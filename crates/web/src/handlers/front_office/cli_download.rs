use std::path::PathBuf;

use actix_files::NamedFile;
use actix_web::{get, web, Responder};

use crate::error::AppError;

/// Registered path to the `batlehub-cli` binary.
///
/// Inserted into `app_data` by `server/src/main.rs` when `[server] cli_binary_path`
/// is configured. Absent in tests and when the operator has not configured a path.
pub struct CliBinaryPath(pub PathBuf);

/// Sanitise a filename for use in a `Content-Disposition: attachment; filename="..."` value.
///
/// Replaces `"`, `\`, non-ASCII characters, and ASCII control characters (including CR/LF)
/// with `_` so the resulting string is safe inside a quoted-string token (RFC 7230 §3.2.6).
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '"' | '\\' => '_',
            c if !c.is_ascii() || c.is_ascii_control() => '_',
            c => c,
        })
        .collect()
}

/// Download the `batlehub-cli` binary.
///
/// Streams the file with `Content-Disposition: attachment` so browsers save it as a file.
/// Uses `actix_files::NamedFile` for zero-copy streaming.
/// Requires `CliBinaryPath` to be registered in app_data (from `[server] cli_binary_path`).
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
    cli_path: Option<web::Data<CliBinaryPath>>,
) -> Result<impl Responder, AppError> {
    let path = cli_path
        .ok_or_else(|| AppError::not_found("No CLI binary has been configured on this server"))?;

    // Offload the blocking open() syscall to the thread pool so the tokio worker
    // is not stalled. NamedFile::open_async does NOT use spawn_blocking internally
    // in actix-files 0.6 (without the experimental-io-uring feature).
    let path_buf = path.0.clone();
    let file = actix_web::web::block(move || NamedFile::open(&path_buf))
        .await
        .map_err(|e| AppError::internal(format!("Failed to open CLI binary: {e}")))?
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AppError::not_found("CLI binary path is configured but the file does not exist")
            } else {
                AppError::internal(format!("Failed to open CLI binary: {e}"))
            }
        })?;

    let raw_name = path
        .0
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("batlehub-cli");
    let safe_name = sanitize_filename(raw_name);

    Ok(file.set_content_disposition(actix_web::http::header::ContentDisposition {
        disposition: actix_web::http::header::DispositionType::Attachment,
        parameters: vec![actix_web::http::header::DispositionParam::Filename(
            safe_name,
        )],
    }))
}
