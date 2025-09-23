use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use thiserror::Error;

pub type AuthResult<T> = Result<T, AuthError>;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("token missing kid header")]
    MissingKeyId,
    #[error("no decoding key registered for kid '{0}'")]
    UnknownKeyId(String),
    #[error("failed to decode token header: {0}")]
    InvalidHeader(String),
    #[error("token verification failed: {0}")]
    Verification(String),
    #[error("invalid claim '{0}' with value '{1}'")]
    InvalidClaim(&'static str, String),
    #[error("malformed claim payload: {0}")]
    InvalidJson(String),
    #[error("failed to parse decoding key for kid '{0}': {1}")]
    KeyParse(String, String),
    #[error("authorization header missing")]
    MissingAuthorization,
    #[error("authorization header malformed")]
    InvalidAuthorization,
    #[error("failed to fetch JWKS: {0}")]
    JwksFetch(String),
    #[error("failed to parse JWKS response: {0}")]
    JwksDecode(String),
    #[error("JWKS entry missing key id (kid)")]
    JwksMissingKid,
    #[error("JWKS key '{0}' missing required RSA components")]
    JwksMissingComponents(String),
    #[error("JWKS key '{kid}' uses unsupported key type '{kty}'")]
    JwksUnsupportedKey { kid: String, kty: String },
    #[error("JWKS key '{kid}' uses unsupported alg '{alg}'")]
    JwksUnsupportedAlg { kid: String, alg: String },
}

impl From<jsonwebtoken::errors::Error> for AuthError {
    fn from(value: jsonwebtoken::errors::Error) -> Self {
        Self::Verification(value.to_string())
    }
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            AuthError::MissingAuthorization | AuthError::InvalidAuthorization => {
                (StatusCode::UNAUTHORIZED, "AUTH_HEADER")
            }
            AuthError::MissingKeyId | AuthError::UnknownKeyId(_) => {
                (StatusCode::UNAUTHORIZED, "AUTH_KEY")
            }
            AuthError::InvalidHeader(_) | AuthError::Verification(_) => {
                (StatusCode::UNAUTHORIZED, "AUTH_TOKEN")
            }
            AuthError::InvalidClaim(_, _)
            | AuthError::InvalidJson(_)
            | AuthError::KeyParse(_, _) => (StatusCode::BAD_REQUEST, "AUTH_CLAIMS"),
            AuthError::JwksFetch(_)
            | AuthError::JwksDecode(_)
            | AuthError::JwksMissingKid
            | AuthError::JwksMissingComponents(_)
            | AuthError::JwksUnsupportedKey { .. }
            | AuthError::JwksUnsupportedAlg { .. } => {
                (StatusCode::INTERNAL_SERVER_ERROR, "AUTH_JWKS")
            }
        };

        let body = ErrorBody {
            code,
            message: self.to_string(),
        };
        (status, Json(body)).into_response()
    }
}
