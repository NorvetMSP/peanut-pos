use axum::http::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("missing tenant identifier")]    MissingTenant,
    #[error("mismatched tenant context")]    MismatchedTenant,
    #[error("unauthorized - missing required role")]    Forbidden,
    #[error("invalid authorization token")]  InvalidToken,
    #[error("internal security error")]      Internal,
}

impl From<SecurityError> for (StatusCode, String) {
    fn from(e: SecurityError) -> Self {
        match e {
            SecurityError::MissingTenant => (StatusCode::BAD_REQUEST, e.to_string()),
            SecurityError::MismatchedTenant => (StatusCode::UNAUTHORIZED, e.to_string()),
            SecurityError::Forbidden => (StatusCode::FORBIDDEN, e.to_string()),
            SecurityError::InvalidToken => (StatusCode::UNAUTHORIZED, e.to_string()),
            SecurityError::Internal => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
}
