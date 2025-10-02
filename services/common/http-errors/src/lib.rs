use axum::{http::{StatusCode, HeaderValue}, response::{IntoResponse, Response}, Json};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize, Debug)]
pub struct ErrorBody {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub missing_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub trace_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")] pub message: Option<String>,
}

#[derive(Debug)]
pub enum ApiError {
    ForbiddenMissingRole { role: &'static str, trace_id: Option<Uuid> },
    Forbidden { trace_id: Option<Uuid> },
    BadRequest { code: &'static str, trace_id: Option<Uuid>, message: Option<String> },
    NotFound { code: &'static str, trace_id: Option<Uuid> },
    Internal { trace_id: Option<Uuid>, message: Option<String> },
}

impl ApiError {
    pub fn internal<E: std::fmt::Display>(e: E, trace_id: Option<Uuid>) -> Self { Self::Internal { trace_id, message: Some(e.to_string()) } }
    pub fn bad_request(code: &'static str, trace_id: Option<Uuid>) -> Self { Self::BadRequest { code, trace_id, message: None } }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body, error_code) = match self {
            ApiError::ForbiddenMissingRole { role, trace_id } => (
                StatusCode::FORBIDDEN,
                ErrorBody { code: "missing_role".into(), missing_role: Some(role.into()), trace_id, message: None },
                "missing_role"
            ),
            ApiError::Forbidden { trace_id } => (
                StatusCode::FORBIDDEN,
                ErrorBody { code: "forbidden".into(), missing_role: None, trace_id, message: None },
                "forbidden"
            ),
            ApiError::BadRequest { code, trace_id, message } => (
                StatusCode::BAD_REQUEST,
                ErrorBody { code: code.into(), missing_role: None, trace_id, message },
                code
            ),
            ApiError::NotFound { code, trace_id } => (
                StatusCode::NOT_FOUND,
                ErrorBody { code: code.into(), missing_role: None, trace_id, message: None },
                code
            ),
            ApiError::Internal { trace_id, message } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorBody { code: "internal_error".into(), missing_role: None, trace_id, message },
                "internal_error"
            ),
        };
        let mut resp = (status, Json(body)).into_response();
        if let Ok(val) = HeaderValue::from_str(error_code) {
            resp.headers_mut().insert("X-Error-Code", val);
        }
        resp
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
