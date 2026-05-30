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
