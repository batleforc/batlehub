use std::path::PathBuf;
use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};

use crate::error::AppError;

/// Registered path to the `batlehub-cli` binary.
///
/// Inserted into `app_data` by `server/src/main.rs` when `[server] cli_binary_path`
/// is configured. Absent in tests and when the operator has not configured a path.
pub struct CliBinaryPath(pub PathBuf);

/// Download the `batlehub-cli` binary.
///
/// Returns the raw binary with `Content-Disposition: attachment` so browsers
/// save it as a file. Requires `CliBinaryPath` to be registered in app_data
/// (done by `server/src/main.rs` from `[server] cli_binary_path`).
/// Returns 404 when no binary has been configured.
#[utoipa::path(
    get,
    path = "/api/v1/cli/download",
    tag = "front-office",
    responses(
        (status = 200, description = "CLI binary (application/octet-stream)"),
        (status = 404, description = "No CLI binary configured on this server"),
    ),
)]
#[get("/api/v1/cli/download")]
pub async fn download_cli(
    cli_path: Option<web::Data<Arc<CliBinaryPath>>>,
) -> Result<impl Responder, AppError> {
    let path = cli_path
        .ok_or_else(|| AppError::not_found("No CLI binary has been configured on this server"))?;

    if !path.0.exists() {
        return Err(AppError::not_found(
            "CLI binary path is configured but the file does not exist",
        ));
    }

    let bytes = tokio::fs::read(&path.0)
        .await
        .map_err(|e| AppError::internal(format!("Failed to read CLI binary: {e}")))?;

    let filename = path
        .0
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("batlehub-cli");

    Ok(HttpResponse::Ok()
        .content_type("application/octet-stream")
        .insert_header((
            "Content-Disposition",
            format!("attachment; filename=\"{filename}\""),
        ))
        .body(bytes))
}
