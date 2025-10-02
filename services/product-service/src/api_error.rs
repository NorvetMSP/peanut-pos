use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
pub struct ErrorBody<'a> {
    pub code: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_role: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<&'a str>,
}

pub enum ApiError {
    ForbiddenMissingRole { role: &'static str, trace_id: Option<Uuid> },
    Forbidden { trace_id: Option<Uuid> },
    BadRequest { code: &'static str, trace_id: Option<Uuid> },
    NotFound { code: &'static str, trace_id: Option<Uuid> },
    Internal { trace_id: Option<Uuid>, message: String },
}

impl ApiError {
    pub fn internal<E: std::fmt::Display>(e: E, trace_id: Option<Uuid>) -> Self {
        ApiError::Internal { trace_id, message: e.to_string() }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::ForbiddenMissingRole { role, trace_id } => (
                StatusCode::FORBIDDEN,
                Json(ErrorBody { code: "missing_role", missing_role: Some(role), trace_id, message: None })
            ).into_response(),
            ApiError::Forbidden { trace_id } => (
                StatusCode::FORBIDDEN,
                Json(ErrorBody { code: "forbidden", missing_role: None, trace_id, message: None })
            ).into_response(),
            ApiError::BadRequest { code, trace_id } => (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody { code, missing_role: None, trace_id, message: None })
            ).into_response(),
            ApiError::NotFound { code, trace_id } => (
                StatusCode::NOT_FOUND,
                Json(ErrorBody { code, missing_role: None, trace_id, message: None })
            ).into_response(),
            ApiError::Internal { trace_id, message } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody { code: "internal_error", missing_role: None, trace_id, message: Some(Box::leak(message.into_boxed_str())) })
            ).into_response(),
        }
    }
}