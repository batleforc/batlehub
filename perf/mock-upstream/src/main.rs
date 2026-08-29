use actix_web::{get, web, App, HttpRequest, HttpResponse, HttpServer};
use clap::Parser;
use rand::RngCore;
use std::time::Duration;

#[derive(Parser, Clone)]
#[command(about = "Mock upstream registry for BatleHub performance tests")]
struct Args {
    #[arg(long, default_value = "9999")]
    port: u16,

    /// Simulated upstream latency in milliseconds
    #[arg(long, default_value = "0")]
    delay_ms: u64,

    /// Fake artifact size in kilobytes
    #[arg(long, default_value = "512")]
    artifact_size_kb: usize,

    /// Packages in the generated RubyGems compact index, and versions of each.
    ///
    /// RFC 0015 §11.7 arm 1 is "today — unfiltered, shared cache", which on this
    /// server is the *proxy* path: the upstream document is cached under an
    /// identity-blind key and served to everyone. That arm needs an upstream
    /// document of the same size as the local corpus arm 2 builds, or the two
    /// numbers describe different documents and comparing them says nothing.
    /// These two flags mirror `corpus-seed --size`.
    #[arg(long, default_value = "1000")]
    gems: usize,

    #[arg(long, default_value = "5")]
    gem_versions: usize,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    println!(
        "mock-upstream listening on :{} delay={}ms artifact={}KB",
        args.port, args.delay_ms, args.artifact_size_kb
    );

    let args = web::Data::new(args);
    let port = args.port;

    HttpServer::new(move || {
        App::new()
            .app_data(args.clone())
            .service(health)
            .service(npm_packument)
            .service(npm_tarball)
            .service(cargo_download)
            .service(cargo_index)
            .service(gem_versions_doc)
            .service(gem_names_doc)
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}

// ── health ────────────────────────────────────────────────────────────────────

#[get("/health")]
async fn health() -> HttpResponse {
    HttpResponse::Ok().body("ok")
}

// ── npm ───────────────────────────────────────────────────────────────────────

/// npm packument: GET /{name}
/// Serves a minimal packument with a single version whose tarball points back
/// to this mock server so the proxy fetches it from here too.
#[get("/npm/{name}")]
async fn npm_packument(
    req: HttpRequest,
    name: web::Path<String>,
    args: web::Data<Args>,
) -> HttpResponse {
    delay(args.delay_ms).await;

    let host = req
        .connection_info()
        .host()
        .to_string();
    let pkg = name.into_inner();
    let version = "1.0.0";
    let tarball_url = format!("http://{}/npm/{}/-/{}-{}.tgz", host, pkg, pkg, version);

    let body = serde_json::json!({
        "name": pkg,
        "dist-tags": { "latest": version },
        "versions": {
            version: {
                "name": pkg,
                "version": version,
                "description": "mock package for perf tests",
                "dist": {
                    "tarball": tarball_url,
                    "shasum": "aabbccdd112233445566778899aabbccdd112233"
                }
            }
        },
        "time": {
            version: "2024-01-01T00:00:00.000Z"
        }
    });

    HttpResponse::Ok()
        .content_type("application/json")
        .body(body.to_string())
}

/// npm tarball: GET /{name}/-/{filename}.tgz
#[get("/npm/{name}/-/{filename}")]
async fn npm_tarball(args: web::Data<Args>) -> HttpResponse {
    delay(args.delay_ms).await;
    HttpResponse::Ok()
        .content_type("application/octet-stream")
        .body(random_bytes(args.artifact_size_kb * 1024))
}

// ── cargo ─────────────────────────────────────────────────────────────────────

/// Sparse cargo index config: GET /cargo/config.json
#[get("/cargo/config.json")]
async fn cargo_index(req: HttpRequest) -> HttpResponse {
    let host = req.connection_info().host().to_string();
    let body = serde_json::json!({
        "dl": format!("http://{}/cargo/{{crate}}/{{version}}/download", host),
        "api": format!("http://{}/cargo", host)
    });
    HttpResponse::Ok()
        .content_type("application/json")
        .body(body.to_string())
}

/// Cargo crate download: GET /cargo/{name}/{version}/download
#[get("/cargo/{name}/{version}/download")]
async fn cargo_download(args: web::Data<Args>) -> HttpResponse {
    delay(args.delay_ms).await;
    HttpResponse::Ok()
        .content_type("application/octet-stream")
        .body(random_bytes(args.artifact_size_kb * 1024))
}

// ── rubygems compact index ────────────────────────────────────────────────────
//
// Generated to order rather than stored, so a corpus size is a flag rather than
// a fixture file: the L corpus's document is ~30 MB and does not belong in git.
//
// The shape is the compact-index format Bundler reads and the one
// `LocalRegistryService::get_rubygems_compact_versions` emits, down to the
// `created_at` epoch and the per-gem MD5 — not because this mock's bytes are
// checked, but because the proxy parses what it caches, and a document it
// cannot parse would measure an error path.

/// `GET /gems/versions` — every gem in the registry, with its live versions.
#[get("/gems/versions")]
async fn gem_versions_doc(args: web::Data<Args>) -> HttpResponse {
    delay(args.delay_ms).await;
    let mut out = String::from("created_at: 1970-01-01T00:00:00Z\n---\n");
    for p in 0..args.gems {
        let name = format!("perf-gem-{p:07}");
        // Every tenth version yanked, matching `corpus-seed`: `/versions` drops
        // them and `/names` does not.
        let live: Vec<String> = (0..args.gem_versions)
            .filter(|v| v % 10 != 9)
            .map(|v| format!("{}.{}.{}", v / 100, (v / 10) % 10, v % 10))
            .collect();
        if live.is_empty() {
            continue;
        }
        out.push_str(&format!("{name} {} {:032x}\n", live.join(","), p as u128));
    }
    HttpResponse::Ok().content_type("text/plain").body(out)
}

/// `GET /gems/names` — the gem names alone.
#[get("/gems/names")]
async fn gem_names_doc(args: web::Data<Args>) -> HttpResponse {
    delay(args.delay_ms).await;
    let mut out = String::from("---\n");
    for p in 0..args.gems {
        out.push_str(&format!("perf-gem-{p:07}\n"));
    }
    HttpResponse::Ok().content_type("text/plain").body(out)
}

// ── helpers ───────────────────────────────────────────────────────────────────

async fn delay(ms: u64) {
    if ms > 0 {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
}

fn random_bytes(size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
}
