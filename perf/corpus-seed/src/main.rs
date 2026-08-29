//! Seed a whole-registry corpus straight into `local_packages`.
//!
//! # Why this exists
//!
//! RFC 0015 §11.7 measures four whole-registry documents — RubyGems' `/versions`
//! and `/names`, the cargo sparse index and conda's `repodata.json` — at three
//! estate sizes:
//!
//! | Size | Packages | Versions | Represents            |
//! | ---- | -------- | -------- | --------------------- |
//! | S    | 1 000    | 5 000    | a team registry       |
//! | M    | 25 000   | 250 000  | an enterprise estate  |
//! | L    | 200 000  | 2 000 000| a public mirror       |
//!
//! `perf/scripts/seed.sh` warms a cache over HTTP, which is right for the
//! existing scenarios and hopeless for these: two million publishes through the
//! full request path would take longer than the measurement is worth, and would
//! measure the publish path rather than the read path it exists to set up.
//!
//! So the corpus goes in through `COPY ... FROM STDIN`, in the *published*
//! state, exactly as `LocalRegistryBackend::publish` would leave it. The read
//! path under measurement cannot tell the difference: it reads
//! `local_packages`, and every column a document builder touches is written
//! here.
//!
//! # What the shape has to be right about
//!
//! Two things, and both change the number:
//!
//! - **Versions per package.** The documents are built by a loop over package
//!   *names* that loads each one's versions, so 25 000 packages × 10 versions
//!   and 250 000 packages × 1 version cost very different amounts for the same
//!   row count. The table above fixes the ratio; `--packages` and
//!   `--versions-per-package` override it for a one-off.
//! - **`index_metadata` size.** It is `JSONB`, it is read for every row of every
//!   document, and a corpus of `{}` would measure a table this server does not
//!   have. Each row carries a realistic RubyGems dependency block and a
//!   checksum, which is what `render_compact_info` reads.
//!
//! # Visibility
//!
//! `--private-fraction` marks that share of packages `internal` rather than
//! `public`. It changes nothing for the unfiltered arm and is the whole point of
//! the filtered one: a document built per identity has to *decide* per package,
//! and a corpus where every package is public lets a filter that never rejects
//! anything look free.

use std::time::Instant;

use clap::Parser;
use futures_util::pin_mut;
use tokio_postgres::types::Type;
use tokio_postgres::{binary_copy::BinaryCopyInWriter, NoTls};

/// One of RFC 0015 §11.7's three corpus sizes, or a hand-rolled one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Size {
    S,
    M,
    L,
}

impl std::str::FromStr for Size {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "s" => Ok(Size::S),
            "m" => Ok(Size::M),
            "l" => Ok(Size::L),
            other => Err(format!("unknown size '{other}' (expected s, m or l)")),
        }
    }
}

impl Size {
    /// `(packages, versions_per_package)`, from §11.7's table.
    fn shape(self) -> (usize, usize) {
        match self {
            Size::S => (1_000, 5),
            Size::M => (25_000, 10),
            Size::L => (200_000, 10),
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "corpus-seed",
    about = "Seed a whole-registry corpus into local_packages for the RFC 0015 §11.7 measurement"
)]
struct Args {
    /// Postgres connection string.
    #[arg(
        long,
        env = "DATABASE_URL",
        default_value = "postgresql://batlehub:changeme@localhost:5432/batlehub"
    )]
    database_url: String,

    /// Registry name to seed into. Must exist in the server's config.
    #[arg(long, default_value = "perf-gems")]
    registry: String,

    /// Corpus size: s, m or l.
    #[arg(long, default_value = "s")]
    size: Size,

    /// Override the package count implied by `--size`.
    #[arg(long)]
    packages: Option<usize>,

    /// Override the versions-per-package implied by `--size`.
    #[arg(long)]
    versions_per_package: Option<usize>,

    /// Fraction of packages marked `internal` rather than `public`, 0.0–1.0.
    ///
    /// Deterministic, not random: every `1/fraction`-th package is internal, so
    /// two runs of the same arguments produce the same corpus and two arms are
    /// compared over identical data.
    #[arg(long, default_value_t = 0.10)]
    private_fraction: f64,

    /// Delete this registry's existing rows first.
    #[arg(long, default_value_t = true)]
    truncate: bool,

    /// Rows per `COPY` batch. Bounds peak memory on the L corpus.
    #[arg(long, default_value_t = 50_000)]
    batch: usize,

    /// Fraction of packages given a package-tier grant, 0.0–1.0.
    ///
    /// RFC 0015 §11.7's resolution number asks what `authorize` costs on a
    /// single coordinate, and the answer depends on whether the `grants` lookup
    /// *finds* anything: an index probe that returns no rows is the cheap case
    /// and the common one, so a corpus with none would measure only the cheap
    /// case and report it as the number.
    ///
    /// Deterministic, like `--private-fraction`: every `1/fraction`-th package
    /// gets one, so two runs compare like with like.
    #[arg(long, default_value_t = 0.10)]
    granted_fraction: f64,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let (packages, per_package) = args.size.shape();
    let packages = args.packages.unwrap_or(packages);
    let per_package = args.versions_per_package.unwrap_or(per_package);
    let total = packages * per_package;

    println!(
        "==> corpus-seed  registry={}  size={:?}  packages={packages}  versions/pkg={per_package}  \
         rows={total}  private={:.0}%",
        args.registry,
        args.size,
        args.private_fraction * 100.0
    );

    let (client, connection) = tokio_postgres::connect(&args.database_url, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {e}");
        }
    });

    if args.truncate {
        let started = Instant::now();
        let removed = client
            .execute(
                "DELETE FROM local_packages WHERE registry = $1",
                &[&args.registry],
            )
            .await?;
        client
            .execute("DELETE FROM grants WHERE registry = $1", &[&args.registry])
            .await?;
        println!(
            "    cleared {removed} existing row(s) in {:.1}s",
            started.elapsed().as_secs_f64()
        );
    }

    // `private_fraction` as "every Nth package", so the corpus is reproducible.
    // 0.0 means never; anything else is at least every package.
    let private_every = if args.private_fraction <= 0.0 {
        usize::MAX
    } else {
        ((1.0 / args.private_fraction).round() as usize).max(1)
    };

    let started = Instant::now();
    let mut written = 0usize;
    let mut pkg = 0usize;

    while pkg < packages {
        let batch_packages = (args.batch / per_package).max(1).min(packages - pkg);
        written += copy_batch(
            &client,
            &args.registry,
            pkg,
            batch_packages,
            per_package,
            private_every,
        )
        .await?;
        pkg += batch_packages;

        let elapsed = started.elapsed().as_secs_f64();
        println!(
            "    {written}/{total} rows  ({:.0} rows/s, {:.1}s elapsed)",
            written as f64 / elapsed.max(0.001),
            elapsed
        );
    }

    // Package-tier grants, for the resolution measurement.
    if args.granted_fraction > 0.0 {
        let every = ((1.0 / args.granted_fraction).round() as usize).max(1);
        let started = Instant::now();
        let written = copy_grants(&client, &args.registry, packages, every).await?;
        println!(
            "    {written} package-tier grant(s) in {:.1}s",
            started.elapsed().as_secs_f64()
        );
    }

    // ANALYZE, not VACUUM FULL: the planner has just had a million rows appear
    // under it, and a measurement taken against stale statistics is a
    // measurement of the wrong query plan.
    println!("    analyzing…");
    client.execute("ANALYZE local_packages", &[]).await?;

    println!(
        "==> seeded {written} rows in {:.1}s",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// `COPY` one batch of packages, every version of each, and answer the row count.
async fn copy_batch(
    client: &tokio_postgres::Client,
    registry: &str,
    first_package: usize,
    packages: usize,
    per_package: usize,
    private_every: usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    let sink = client
        .copy_in(
            "COPY local_packages \
             (registry, name, version, checksum, yanked, index_metadata, published_by, \
              status, visibility) \
             FROM STDIN BINARY",
        )
        .await?;

    let types = [
        Type::TEXT,
        Type::TEXT,
        Type::TEXT,
        Type::TEXT,
        Type::BOOL,
        Type::JSONB,
        Type::TEXT,
        Type::TEXT,
        Type::TEXT,
    ];
    let writer = BinaryCopyInWriter::new(sink, &types);
    pin_mut!(writer);

    let mut rows = 0usize;
    for p in first_package..first_package + packages {
        // Zero-padded so lexical and numeric order agree; a document builder
        // that sorts by name then sees the order it would see in production.
        let name = format!("perf-gem-{p:07}");
        let visibility = if p % private_every == 0 {
            "internal"
        } else {
            "public"
        };

        for v in 0..per_package {
            let version = format!("{}.{}.{}", v / 100, (v / 10) % 10, v % 10);
            let checksum = format!("{:064x}", (p as u128) << 16 | v as u128);
            let meta = index_metadata(&name, &version, &checksum, p);
            // Every tenth version yanked: `/versions` filters them out and
            // `/names` does not, so a corpus with none would hide the
            // difference between the two documents.
            let yanked = v % 10 == 9;

            writer
                .as_mut()
                .write(&[
                    &registry,
                    &name,
                    &version,
                    &checksum,
                    &yanked,
                    &meta,
                    &"perf-seed",
                    &"published",
                    &visibility,
                ])
                .await?;
            rows += 1;
        }
    }

    writer.finish().await?;
    Ok(rows)
}

/// One `grants` row per `every`-th package.
///
/// `user:perf-user` so the perf identity actually resolves them — a grant nobody
/// matches is an index probe that finds a row and then discards it, which
/// measures the lookup but not the resolution.
async fn copy_grants(
    client: &tokio_postgres::Client,
    registry: &str,
    packages: usize,
    every: usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    let sink = client
        .copy_in(
            "COPY grants (registry, node_kind, node_key, subject, actions, granted_by) \
             FROM STDIN BINARY",
        )
        .await?;
    let types = [
        Type::TEXT,
        Type::TEXT,
        Type::TEXT,
        Type::TEXT,
        Type::TEXT_ARRAY,
        Type::TEXT,
    ];
    let writer = BinaryCopyInWriter::new(sink, &types);
    pin_mut!(writer);

    let actions: Vec<String> = vec!["releases:read".to_owned(), "releases:list".to_owned()];
    let mut rows = 0usize;
    for p in (0..packages).step_by(every) {
        let name = format!("perf-gem-{p:07}");
        writer
            .as_mut()
            .write(&[
                &registry,
                &"package",
                &name,
                &"user:perf-user",
                &actions,
                &"corpus-seed",
            ])
            .await?;
        rows += 1;
    }
    writer.finish().await?;
    Ok(rows)
}

/// A realistic RubyGems index line.
///
/// `render_compact_info` reads `dependencies[].name`, `dependencies[].requirement`
/// and `sha`; the rest is padding that makes the row the size a real one is,
/// because `index_metadata` is `JSONB` and every document read deserialises it.
fn index_metadata(name: &str, version: &str, checksum: &str, seed: usize) -> serde_json::Value {
    let deps: Vec<serde_json::Value> = (0..4)
        .map(|d| {
            serde_json::json!({
                "name": format!("perf-gem-{:07}", (seed * 7 + d * 13) % 1_000),
                "requirement": ">= 0.0.0",
                "type": "runtime",
            })
        })
        .collect();

    serde_json::json!({
        "name": name,
        "version": { "version": version },
        "platform": "ruby",
        "sha": checksum,
        "dependencies": deps,
        "summary": "corpus-seed fixture package for the RFC 0015 §11.7 measurement",
        "authors": ["perf-seed"],
        "licenses": ["Apache-2.0"],
        // conda's `repodata.json` keys on these two, so the same corpus can be
        // pointed at a conda registry without a second seeder.
        "filename": format!("{name}-{version}-py311_0.conda"),
        "subdir": "linux-64",
    })
}
