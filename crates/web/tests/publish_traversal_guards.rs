//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, TestRequest};

use batlehub_config::schema::RegistryMode;

// ── pypi publish traversal ─────────────────────────────────────────────────────

async fn make_local_pypi_app(
    mode: RegistryMode,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    build_local_registry_app(
        local_registry_app_parts("local-pypi", "pypi", mode, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await
}

/// Build a `twine upload`-style `multipart/form-data` body for `pypi_publish`.
fn make_pypi_publish_body(name: &str, version: &str) -> (Vec<u8>, String) {
    let boundary = "pypiboundary";
    let mut body = Vec::new();
    for (field_name, value) in [
        (":action", "file_upload"),
        ("name", name),
        ("version", version),
    ] {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{field_name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"content\"; filename=\"{name}-{version}.tar.gz\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(b"fake-pypi-sdist-content");
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (body, format!("multipart/form-data; boundary={boundary}"))
}

#[actix_web::test]
async fn pypi_publish_traversal_version_returns_400() {
    let app = make_local_pypi_app(RegistryMode::Local).await;
    let (body, content_type) = make_pypi_publish_body("my-pkg", "../../etc/x");
    let req = TestRequest::post()
        .uri("/proxy/local-pypi/legacy/")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", content_type))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

// ── conda publish traversal ─────────────────────────────────────────────────────

/// Minimal conda `.tar.bz2` package: a bzip2-compressed tar containing
/// `info/index.json`.
fn make_conda_tar_bz2(name: &str, version: &str) -> Vec<u8> {
    use bzip2::write::BzEncoder;
    use bzip2::Compression;
    use std::io::Write as _;

    let index_json = serde_json::json!({
        "name": name,
        "version": version,
        "build": "0",
        "build_number": 0,
        "depends": [],
        "subdir": "linux-64",
    });
    let index_bytes = serde_json::to_vec(&index_json).unwrap();

    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        let mut header = tar::Header::new_gnu();
        header.set_size(index_bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "info/index.json", index_bytes.as_slice())
            .unwrap();
        builder.finish().unwrap();
    }

    let mut encoder = BzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&tar_bytes).unwrap();
    encoder.finish().unwrap()
}

async fn make_local_conda_app(
    mode: RegistryMode,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    build_local_registry_app(
        local_registry_app_parts("local-conda", "conda", mode, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await
}

#[actix_web::test]
async fn conda_publish_traversal_version_returns_400() {
    let app = make_local_conda_app(RegistryMode::Local).await;
    let req = TestRequest::post()
        .uri("/proxy/local-conda/linux-64/")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_conda_tar_bz2("my-pkg", "../../etc/x"))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}
