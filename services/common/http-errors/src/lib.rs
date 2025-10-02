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

// Shared HTTP error metrics middleware helper
use once_cell::sync::Lazy;
use prometheus::{IntCounterVec, Opts};
use axum::{body::Body, http::Request};
use axum::middleware::Next;

static HTTP_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new(
            "http_errors_total",
            "Count of HTTP error responses emitted (status >= 400)",
        ),
        &["service", "code", "status"],
    ).expect("http_errors_total");
    let _ = prometheus::default_registry().register(Box::new(c.clone()));
    c
});

/// Returns an Axum middleware function that records HTTP error counts.
/// Usage: .layer(axum::middleware::from_fn(http_error_metrics_layer("service-name")))
pub fn http_error_metrics_layer(service_name: &'static str) -> impl Fn(Request<Body>, Next) -> std::pin::Pin<Box<dyn std::future::Future<Output=Result<axum::response::Response, ApiError>> + Send>> + Clone + Send + Sync + 'static {
    move |req: Request<Body>, next: Next| {
        let svc = service_name;
        Box::pin(async move {
            let resp = next.run(req).await;
            let status = resp.status();
            if status.as_u16() >= 400 {
                let code = resp.headers().get("X-Error-Code").and_then(|v| v.to_str().ok()).unwrap_or("unknown");
                HTTP_ERRORS_TOTAL.with_label_values(&[svc, code, status.as_str()]).inc();
            }
            Ok(resp)
        })
    }
}

#[cfg(any(test, feature = "test-helpers"))]
pub mod test_helpers {
    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;

    // Simple helper (not a macro) to assert error shape from an ApiError directly.
    pub async fn assert_error_shape(err: ApiError, expected_code: &str) {
        let resp = err.into_response();
        let status = resp.status();
        let headers = resp.headers().clone();
        let body_bytes = to_bytes(resp.into_body(), 1024 * 64).await.expect("read body");
        let text = String::from_utf8(body_bytes.to_vec()).expect("utf8 body");
        assert!(text.contains(&format!("\"code\":\"{}\"", expected_code)), "body missing expected code: {} in {}", expected_code, text);
        assert!(headers.get("X-Error-Code").is_some(), "missing X-Error-Code header");
        if expected_code == "internal_error" {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
}

/// Test-only assertion macro for validating an ApiError's rendered response structure.
/// Usage:
/// assert_api_error!(err, "missing_role");
#[cfg(any(test, feature = "test-helpers"))]
#[macro_export]
macro_rules! assert_api_error {
    ($err:expr, $code:expr) => {{
        let err: $crate::ApiError = $err; // type ascription if inference ambiguous
        $crate::test_helpers::assert_error_shape(err, $code).await;
    }};
}
