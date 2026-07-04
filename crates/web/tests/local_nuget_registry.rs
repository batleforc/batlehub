//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body, read_body_json, TestRequest};
use serde_json::Value;

use batlehub_config::schema::RegistryMode;

// ══ NuGet local registry tests ════════════════════════════════════════════════

/// Build a minimal in-memory .nupkg (ZIP) containing a .nuspec with the given id/version.
fn make_sample_nupkg(id: &str, version: &str, description: &str) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    let nuspec = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd">
  <metadata>
    <id>{id}</id>
    <version>{version}</version>
    <description>{description}</description>
    <authors>TestAuthor</authors>
    <tags>test</tags>
  </metadata>
</package>"#
    );

    let mut buf = Vec::new();
    let mut zip = ZipWriter::new(std::io::Cursor::new(&mut buf));
    let opts = SimpleFileOptions::default();
    zip.start_file(format!("{id}.nuspec"), opts).unwrap();
    zip.write_all(nuspec.as_bytes()).unwrap();
    zip.finish().unwrap();
    buf
}

/// Wrap a .nupkg in a `multipart/form-data` body and return `(body_bytes, content_type_header)`.
fn make_nuget_publish_body(nupkg: &[u8]) -> (Vec<u8>, String) {
    let boundary = "nugetboundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"package.nupkg\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    ).into_bytes();
    body.extend_from_slice(nupkg);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let ct = format!("multipart/form-data; boundary={boundary}");
    (body, ct)
}

#[actix_web::test]
async fn nuget_service_index_returns_valid_json() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/index.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["version"], "3.0.0");
    assert!(body["resources"]
        .as_array()
        .map(|a| !a.is_empty())
        .unwrap_or(false));
}

#[actix_web::test]
async fn nuget_service_index_includes_vulnerabilities_url_resource() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/index.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let resources = body["resources"]
        .as_array()
        .expect("resources must be an array");
    let vuln_resource = resources
        .iter()
        .find(|r| r["@type"].as_str() == Some("VulnerabilitiesUrl/6.7.0"))
        .expect("service index must contain a VulnerabilitiesUrl/6.7.0 resource");
    let id = vuln_resource["@id"].as_str().expect("@id must be a string");
    assert!(
        id.contains("/proxy/local-nuget/nuget/v3/vulnerabilities/"),
        "@id must point to this server's vulnerability endpoint, got: {id}"
    );
}

#[actix_web::test]
async fn nuget_publish_creates_version() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let nupkg = make_sample_nupkg("MyLib", "1.0.0", "A test library");
    let (body, ct) = make_nuget_publish_body(&nupkg);

    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", ct))
        .set_payload(body)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 201);

    // Version should now appear in flat container
    let req2 = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/flat/mylib/index.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp2 = call_service(&app, req2).await;
    assert_eq!(resp2.status(), 200);
    let body2: Value = read_body_json(resp2).await;
    let versions = body2["versions"].as_array().unwrap();
    assert!(versions.iter().any(|v| v == "1.0.0"));
}

#[actix_web::test]
async fn nuget_publish_requires_auth() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let nupkg = make_sample_nupkg("MyLib", "1.0.0", "Test");
    let (body, ct) = make_nuget_publish_body(&nupkg);

    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Content-Type", ct))
        .set_payload(body)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn nuget_publish_duplicate_returns_409() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let nupkg = make_sample_nupkg("MyLib", "1.0.0", "Test");

    let (body1, ct1) = make_nuget_publish_body(&nupkg);
    let req1 = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", ct1))
        .set_payload(body1)
        .to_request();
    assert_eq!(call_service(&app, req1).await.status(), 201);

    let (body2, ct2) = make_nuget_publish_body(&nupkg);
    let req2 = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", ct2))
        .set_payload(body2)
        .to_request();
    assert_eq!(call_service(&app, req2).await.status(), 409);
}

#[actix_web::test]
async fn nuget_xnuget_apikey_header_authenticates() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let nupkg = make_sample_nupkg("KeyLib", "0.1.0", "Test ApiKey auth");
    let (body, ct) = make_nuget_publish_body(&nupkg);

    // Use X-NuGet-ApiKey instead of Authorization: Bearer
    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("X-NuGet-ApiKey", ADMIN_TOKEN))
        .insert_header(("Content-Type", ct))
        .set_payload(body)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(
        resp.status(),
        201,
        "X-NuGet-ApiKey should authenticate like Bearer"
    );
}

#[actix_web::test]
async fn nuget_yank_removes_from_versions() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let nupkg = make_sample_nupkg("YankLib", "2.0.0", "Yank test");
    let (body, ct) = make_nuget_publish_body(&nupkg);

    // Publish first
    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", ct))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 201);

    // Yank it
    let req_yank = TestRequest::delete()
        .uri("/proxy/local-nuget/nuget/v2/package/yanklib/2.0.0")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req_yank).await.status(), 204);

    // Versions list should be empty (yanked packages are excluded)
    let req_list = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/flat/yanklib/index.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp_list = call_service(&app, req_list).await;
    assert_eq!(resp_list.status(), 200);
    let body_list: Value = read_body_json(resp_list).await;
    let versions = body_list["versions"].as_array().unwrap();
    assert!(
        versions.is_empty(),
        "yanked version should not appear in flat container versions list"
    );
}

#[actix_web::test]
async fn nuget_registration_local_has_catalog_entry() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let nupkg = make_sample_nupkg("RegLib", "1.0.0", "Registration test");
    let (body, ct) = make_nuget_publish_body(&nupkg);

    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", ct))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 201);

    let req2 = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/registration5/reglib/index.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp2 = call_service(&app, req2).await;
    assert_eq!(resp2.status(), 200);
    let body2: Value = read_body_json(resp2).await;
    assert!(body2["count"].as_u64().unwrap_or(0) >= 1);
    let items = body2["items"].as_array().unwrap();
    assert!(!items.is_empty());
    let leaf_items = items[0]["items"].as_array().unwrap();
    assert!(!leaf_items.is_empty());
    let entry = &leaf_items[0]["catalogEntry"];
    assert_eq!(entry["version"], "1.0.0");
}

#[actix_web::test]
async fn nuget_publish_proxy_mode_returns_404() {
    let app = make_local_nuget_app(RegistryMode::Proxy).await;
    let nupkg = make_sample_nupkg("PxLib", "1.0.0", "test");
    let (body, ct) = make_nuget_publish_body(&nupkg);

    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", ct))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn nuget_publish_traversal_id_returns_400() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    // A traversal sequence in the package ID (inside the .nuspec) must be rejected
    // by validate_package_name before reaching the storage layer — clean 400.
    let nupkg = make_sample_nupkg("../../etc/x", "1.0.0", "traversal test");
    let (body, ct) = make_nuget_publish_body(&nupkg);
    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", ct))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn nuget_publish_traversal_version_returns_400() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let nupkg = make_sample_nupkg("SafeLib", "../../etc/x", "traversal test");
    let (body, ct) = make_nuget_publish_body(&nupkg);
    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", ct))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn nuget_flat_download_local_returns_nupkg() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let nupkg = make_sample_nupkg("DlLib", "1.0.0", "Download test");
    let (body, ct) = make_nuget_publish_body(&nupkg);

    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", ct))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 201);

    let req_dl = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/flat/dllib/1.0.0/dllib.1.0.0.nupkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp_dl = call_service(&app, req_dl).await;
    assert_eq!(resp_dl.status(), 200);
    let bytes = read_body(resp_dl).await;
    assert!(
        !bytes.is_empty(),
        "nupkg download should return artifact bytes"
    );
}

#[actix_web::test]
async fn nuget_flat_download_local_returns_nuspec() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let nupkg = make_sample_nupkg("NuspecLib", "2.0.0", "Nuspec extract test");
    let (body, ct) = make_nuget_publish_body(&nupkg);

    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", ct))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 201);

    let req_nuspec = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/flat/nuspeclib/2.0.0/nuspeclib.2.0.0.nuspec")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp_nuspec = call_service(&app, req_nuspec).await;
    assert_eq!(resp_nuspec.status(), 200);
    let body_bytes = read_body(resp_nuspec).await;
    let xml = std::str::from_utf8(&body_bytes).unwrap();
    assert!(
        xml.contains("<id>NuspecLib</id>"),
        "nuspec should contain the package id"
    );
    assert!(
        xml.contains("<version>2.0.0</version>"),
        "nuspec should contain the version"
    );
}

#[actix_web::test]
async fn nuget_search_local_returns_packages() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let nupkg = make_sample_nupkg("SearchMe", "1.0.0", "Search test");
    let (body, ct) = make_nuget_publish_body(&nupkg);

    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", ct))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 201);

    let req2 = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/query?q=search")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp2 = call_service(&app, req2).await;
    assert_eq!(resp2.status(), 200);
    let body2: Value = read_body_json(resp2).await;
    assert!(
        body2["totalHits"].as_u64().unwrap_or(0) >= 1,
        "search should return at least one hit"
    );
}

#[actix_web::test]
async fn nuget_flat_download_missing_returns_404() {
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/flat/ghost/9.9.9/ghost.9.9.9.nupkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}
