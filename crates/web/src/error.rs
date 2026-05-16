use actix_web::HttpResponse;
use actix_web::http::StatusCode;
use serde::Serialize;

use proxy_cache_core::error::CoreError;

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

pub struct AppError {
    pub status: StatusCode,
    pub message: String,
}

impl AppError {
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::NOT_FOUND, message: msg.into() }
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::FORBIDDEN, message: msg.into() }
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: msg.into() }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: msg.into() }
    }
}

impl actix_web::ResponseError for AppError {
    fn status_code(&self) -> StatusCode {
        self.status
    }

    fn error_response(&self) -> HttpResponse {
        let body = ErrorBody {
            error: self.status.canonical_reason().unwrap_or("error").to_owned(),
            message: self.message.clone(),
        };
        HttpResponse::build(self.status).json(body)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}

impl std::fmt::Debug for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppError({} {})", self.status, self.message)
    }
}

impl From<CoreError> for AppError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::NotFound(msg) => Self::not_found(msg),
            CoreError::AccessDenied(msg) => Self::forbidden(msg),
            CoreError::UnknownRegistry(name) => Self::bad_request(format!("unknown registry: {name}")),
            other => {
                tracing::error!(error = %other, "internal error");
                Self::internal("internal server error")
            }
        }
    }
}
