use actix_web::HttpResponse;
use actix_web::http::StatusCode;
use serde::Serialize;

use batlehub_core::error::CoreError;

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

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::CONFLICT, message: msg.into() }
    }

    pub fn unprocessable(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::UNPROCESSABLE_ENTITY, message: msg.into() }
    }

    pub fn service_unavailable(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::SERVICE_UNAVAILABLE, message: msg.into() }
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
            CoreError::Conflict(msg) => Self::conflict(msg),
            CoreError::PayloadTooLarge(msg) => Self {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: msg,
            },
            CoreError::QuotaExceeded(msg) => Self {
                status: StatusCode::TOO_MANY_REQUESTS,
                message: msg,
            },
            CoreError::InvalidVersion(msg) => Self::unprocessable(msg),
            CoreError::Registry(msg) => Self {
                status: StatusCode::BAD_GATEWAY,
                message: msg,
            },
            // Dependency unavailability → 503 so load-balancers can retry elsewhere.
            CoreError::Storage(msg) | CoreError::Cache(msg) => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: msg,
            },
            CoreError::Database(msg) => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: msg,
            },
            other => {
                tracing::error!(error = %other, "internal error");
                Self::internal("internal server error")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_format() {
        let e = AppError::not_found("pkg missing");
        assert!(format!("{e}").contains("pkg missing"));
    }

    #[test]
    fn debug_format() {
        let e = AppError::forbidden("denied");
        assert!(format!("{e:?}").contains("403"));
    }

    #[test]
    fn internal_method() {
        let e = AppError::internal("oops");
        assert_eq!(e.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(e.message, "oops");
    }

    #[test]
    fn from_core_payload_too_large() {
        let e = AppError::from(CoreError::PayloadTooLarge("too big".into()));
        assert_eq!(e.status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn from_core_database_error_maps_to_503() {
        let e = AppError::from(CoreError::Database("db error".into()));
        assert_eq!(e.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn from_core_storage_error_maps_to_503() {
        let e = AppError::from(CoreError::Storage("backend down".into()));
        assert_eq!(e.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn from_core_cache_error_maps_to_503() {
        let e = AppError::from(CoreError::Cache("cache unavailable".into()));
        assert_eq!(e.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn from_core_not_found() {
        let e = AppError::from(CoreError::NotFound("missing".into()));
        assert_eq!(e.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn from_core_conflict() {
        let e = AppError::from(CoreError::Conflict("dup".into()));
        assert_eq!(e.status, StatusCode::CONFLICT);
    }
}
