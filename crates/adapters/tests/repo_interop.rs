//! Generator for the apt/dnf interop harness (`tests/interop/verify.sh`).
//!
//! This `#[ignore]`d test writes a fully-signed Debian APT repo and RPM/YUM repo
//! to `$REPO_INTEROP_OUT` using the **production** signing + index-generation code
//! (`batlehub_adapters::repo` + `OpenPgpSigner`). The shell harness then points
//! real `apt` and `dnf` at the output to confirm they accept BatleHub's hand-rolled
//! Ed25519 OpenPGP signatures and generated metadata.
//!
//! Run via `task test:repo-interop` (not part of the normal `cargo test` run, as it
//! is `#[ignore]`d and requires a writable `$REPO_INTEROP_OUT`).

use std::io::Write;
use std::path::Path;

// `repo_rpm` is BatleHub's repodata generator; the bare `rpm` path is the external
// crate used only to build a fixture package.
use batlehub_adapters::repo::{deb, gzip, pacman, rpm as repo_rpm, OpenPgpSigner};

// A throwaway 32-byte Ed25519 seed — interop fixtures only, never a real key.
const SEED: &str = "9d61b19deffeba00aa3f3b6e3b0fe6a3f3a76b08e2c0a3f3b6e3b0fe6a3f3a76";
const USER_ID: &str = "BatleHub Interop <interop@batlehub.test>";

fn write_file(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
}

fn gzip_bytes(raw: &[u8]) -> Vec<u8> {
    let mut gz = Vec::new();
    let mut enc = flate2::write::GzEncoder::new(&mut gz, flate2::Compression::default());
    enc.write_all(raw).unwrap();
    enc.finish().unwrap();
    gz
}

/// Gzip a single-file tar (used for `control.tar.gz`).
fn tar_gz(path: &str, mode: u32, contents: &[u8]) -> Vec<u8> {
    let mut tar_buf = Vec::new();
    {
        let mut tb = tar::Builder::new(&mut tar_buf);
        let mut h = tar::Header::new_gnu();
        h.set_path(path).unwrap();
        h.set_size(contents.len() as u64);
        h.set_mode(mode);
        h.set_cksum();
        tb.append(&h, contents).unwrap();
        tb.finish().unwrap();
    }
    gzip_bytes(&tar_buf)
}

/// Build `data.tar.gz` with the parent directory entries `dpkg` needs, then one
/// regular file.
fn data_tar_gz(dirs: &[&str], file: &str, contents: &[u8]) -> Vec<u8> {
    let mut tar_buf = Vec::new();
    {
        let mut tb = tar::Builder::new(&mut tar_buf);
        for dir in dirs {
            let mut h = tar::Header::new_gnu();
            h.set_path(dir).unwrap();
            h.set_entry_type(tar::EntryType::Directory);
            h.set_size(0);
            h.set_mode(0o755);
            h.set_cksum();
            tb.append(&h, std::io::empty()).unwrap();
        }
        let mut h = tar::Header::new_gnu();
        h.set_path(file).unwrap();
        h.set_size(contents.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tb.append(&h, contents).unwrap();
        tb.finish().unwrap();
    }
    gzip_bytes(&tar_buf)
}

/// Build a minimal but **installable** `.deb` (ar: debian-binary + control.tar.gz
/// + data.tar.gz with one file so `dpkg` can unpack it).
fn build_deb(control: &str) -> Vec<u8> {
    let control_gz = tar_gz("./control", 0o644, control.as_bytes());
    let data_gz = data_tar_gz(
        &[
            "./usr/",
            "./usr/share/",
            "./usr/share/doc/",
            "./usr/share/doc/hello-batlehub/",
        ],
        "./usr/share/doc/hello-batlehub/README",
        b"BatleHub interop test package\n",
    );
    let mut deb = Vec::new();
    {
        let mut b = ar::Builder::new(&mut deb);
        let dbin = b"2.0\n";
        b.append(
            &ar::Header::new(b"debian-binary".to_vec(), dbin.len() as u64),
            &dbin[..],
        )
        .unwrap();
        b.append(
            &ar::Header::new(b"control.tar.gz".to_vec(), control_gz.len() as u64),
            &control_gz[..],
        )
        .unwrap();
        b.append(
            &ar::Header::new(b"data.tar.gz".to_vec(), data_gz.len() as u64),
            &data_gz[..],
        )
        .unwrap();
    }
    deb
}

/// Build an installable `.rpm` with a plain (non-script) payload file, so RPM does
/// not synthesise a `/bin/sh` dependency that offline `dnf` could not resolve.
fn build_rpm() -> Vec<u8> {
    let pkg = rpm::PackageBuilder::new("hello-batlehub", "1.0", "MIT", "x86_64", "interop test")
        .with_file_contents(
            b"BatleHub interop test package\n".to_vec(),
            rpm::FileOptions::new("/usr/share/hello-batlehub/data.txt").mode(0o100644),
        )
        .unwrap()
        .build()
        .unwrap();
    let mut buf = Vec::new();
    pkg.write(&mut buf).unwrap();
    buf
}

/// Build an installable `.pkg.tar.zst`: a zstd-compressed tar with the root
/// `.PKGINFO` metadata (read first, as `makepkg` writes it) plus one regular file
/// and its parent dirs, so `pacman` can unpack it.
fn build_pacman_pkg(pkginfo: &str) -> Vec<u8> {
    let mut tar_buf = Vec::new();
    {
        let mut tb = tar::Builder::new(&mut tar_buf);
        let mut h = tar::Header::new_gnu();
        h.set_path(".PKGINFO").unwrap();
        h.set_size(pkginfo.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tb.append(&h, pkginfo.as_bytes()).unwrap();

        for dir in ["usr/", "usr/share/", "usr/share/hello-batlehub/"] {
            let mut h = tar::Header::new_gnu();
            h.set_path(dir).unwrap();
            h.set_entry_type(tar::EntryType::Directory);
            h.set_size(0);
            h.set_mode(0o755);
            h.set_cksum();
            tb.append(&h, std::io::empty()).unwrap();
        }
        let contents = b"BatleHub interop test package\n";
        let mut h = tar::Header::new_gnu();
        h.set_path("usr/share/hello-batlehub/data.txt").unwrap();
        h.set_size(contents.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tb.append(&h, &contents[..]).unwrap();
        tb.finish().unwrap();
    }
    zstd::encode_all(std::io::Cursor::new(tar_buf), 0).unwrap()
}

#[test]
#[ignore = "interop generator; run via task test:repo-interop"]
fn generate_signed_repos() {
    let out =
        std::env::var("REPO_INTEROP_OUT").expect("set REPO_INTEROP_OUT to the output directory");
    let out = Path::new(&out);
    let signer = OpenPgpSigner::from_seed_hex(SEED, 1_700_000_000, USER_ID).unwrap();

    // ── Debian APT repo ──────────────────────────────────────────────────────
    let apt = out.join("apt");
    let control = "Package: hello-batlehub\nVersion: 1.0\nArchitecture: amd64\nMaintainer: BatleHub <interop@batlehub.test>\nDescription: interop test package\n";
    let deb_bytes = build_deb(control);
    let pkg = deb::parse_deb(&deb_bytes).unwrap();
    let pool = deb::pool_path("main", &pkg).unwrap();
    write_file(&apt, &pool, &deb_bytes);

    let stanza = deb::packages_stanza(&pkg, &pool);
    let packages = deb::generate_packages(&[stanza]).into_bytes();
    let packages_gz = gzip(&packages).unwrap();
    write_file(&apt, "dists/stable/main/binary-amd64/Packages", &packages);
    write_file(
        &apt,
        "dists/stable/main/binary-amd64/Packages.gz",
        &packages_gz,
    );

    let files = vec![
        deb::ReleaseFile::new("main/binary-amd64/Packages", &packages),
        deb::ReleaseFile::new("main/binary-amd64/Packages.gz", &packages_gz),
    ];
    let arches = vec!["amd64".to_string()];
    let comps = vec!["main".to_string()];
    let meta = deb::ReleaseMeta {
        origin: "BatleHub",
        label: "BatleHub",
        suite: "stable",
        codename: "stable",
        architectures: &arches,
        components: &comps,
        date: "Thu, 01 Jan 1970 00:00:00 UTC",
    };
    let release = deb::generate_release(&meta, &files);
    write_file(&apt, "dists/stable/Release", release.as_bytes());
    write_file(
        &apt,
        "dists/stable/InRelease",
        signer.clear_sign(&release).as_bytes(),
    );
    write_file(
        &apt,
        "dists/stable/Release.gpg",
        signer.detached_sign(release.as_bytes()).as_bytes(),
    );
    write_file(&apt, "key.asc", signer.armored_public_key().as_bytes());

    // ── RPM / YUM repo ───────────────────────────────────────────────────────
    let yum = out.join("yum");
    let rpm_bytes = build_rpm();
    let location = "packages/hello-batlehub-1.0-1.x86_64.rpm";
    let rpkg = repo_rpm::parse_rpm(&rpm_bytes, location).unwrap();
    write_file(&yum, location, &rpm_bytes);

    let primary = repo_rpm::primary_xml(std::slice::from_ref(&rpkg)).into_bytes();
    let filelists = repo_rpm::filelists_xml(std::slice::from_ref(&rpkg)).into_bytes();
    let other = repo_rpm::other_xml(std::slice::from_ref(&rpkg)).into_bytes();
    let mut entries = Vec::new();
    for (kind, plain) in [
        ("primary", &primary),
        ("filelists", &filelists),
        ("other", &other),
    ] {
        let gz = gzip(plain).unwrap();
        let href = format!("repodata/{kind}.xml.gz");
        write_file(&yum, &href, &gz);
        entries.push(repo_rpm::RepoMdData::new(
            kind,
            &href,
            &gz,
            plain,
            1_700_000_000,
        ));
    }
    let repomd = repo_rpm::repomd_xml(&entries).into_bytes();
    write_file(&yum, "repodata/repomd.xml", &repomd);
    write_file(
        &yum,
        "repodata/repomd.xml.asc",
        signer.detached_sign(&repomd).as_bytes(),
    );
    write_file(
        &yum,
        "repodata/repomd.xml.key",
        signer.armored_public_key().as_bytes(),
    );

    // ── Arch Linux pacman repo ───────────────────────────────────────────────
    // Mirrors exactly what the `pacman_publish` handler writes: the package under
    // `{arch}/`, a detached `.sig`, the base64 `%PGPSIG%` embedded in the DB desc,
    // the signed `{repo}.db`, and the armored public key for `pacman-key --add`.
    use base64::Engine;
    let pac = out.join("pacman");
    let arch = "x86_64";
    let repo = "batlehub"; // must match the `[batlehub]` section / `{repo}.db` name
    let pkginfo = concat!(
        "pkgname = hello-batlehub\n",
        "pkgbase = hello-batlehub\n",
        "pkgver = 1.0-1\n",
        "pkgdesc = interop test package\n",
        "url = https://batlehub.test\n",
        "builddate = 1700000000\n",
        "packager = BatleHub <interop@batlehub.test>\n",
        "size = 30\n",
        "arch = x86_64\n",
        "license = MIT\n",
    );
    let pkg_bytes = build_pacman_pkg(pkginfo);
    let filename = format!(
        "hello-batlehub-1.0-1-{arch}{}",
        pacman::download_suffix(&pkg_bytes)
    );
    let mut pkg = pacman::parse_pacman(&pkg_bytes, &filename).unwrap();
    write_file(&pac, &format!("{arch}/{filename}"), &pkg_bytes);

    let pkg_sig = signer.detached_sign_binary(&pkg_bytes);
    write_file(&pac, &format!("{arch}/{filename}.sig"), &pkg_sig);
    pkg.pgpsig = Some(base64::engine::general_purpose::STANDARD.encode(&pkg_sig));

    let db = pacman::generate_db(&[(pacman::db_dir_name(&pkg).unwrap(), pacman::desc_entry(&pkg))])
        .unwrap();
    for name in [
        format!("{repo}.db"),
        format!("{repo}.db.tar.gz"),
        format!("{repo}.files"),
        format!("{repo}.files.tar.gz"),
    ] {
        write_file(&pac, &format!("{arch}/{name}"), &db);
    }
    let db_sig = signer.detached_sign_binary(&db);
    for name in [format!("{repo}.db.sig"), format!("{repo}.files.sig")] {
        write_file(&pac, &format!("{arch}/{name}"), &db_sig);
    }
    write_file(&pac, "key.gpg", signer.armored_public_key().as_bytes());

    eprintln!("wrote signed repos to {}", out.display());
}
