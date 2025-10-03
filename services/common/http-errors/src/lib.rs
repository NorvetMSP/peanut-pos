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
use prometheus::{IntCounterVec, Opts, IntCounter, IntGauge};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashSet;
use std::sync::Mutex;
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

static HTTP_ERROR_CODE_OVERFLOW_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new(
        "http_error_code_overflow_total",
        "Count of error responses whose original code label was replaced by overflow due to cardinality guard",
    ).expect("http_error_code_overflow_total");
    let _ = prometheus::default_registry().register(Box::new(c.clone()));
    c
});

static HTTP_ERROR_CODES_DISTINCT: Lazy<IntGauge> = Lazy::new(|| {
    let g = IntGauge::new(
        "http_error_codes_distinct",
        "Current number of distinct HTTP error codes being tracked (capped by guard)",
    ).expect("http_error_codes_distinct");
    let _ = prometheus::default_registry().register(Box::new(g.clone()));
    g
});

static HTTP_ERROR_CODE_SATURATION: Lazy<IntGauge> = Lazy::new(|| {
    let g = IntGauge::new(
        "http_error_code_saturation",
        "Fraction * 100 (integer percent) of distinct error code capacity used",
    ).expect("http_error_code_saturation");
    let _ = prometheus::default_registry().register(Box::new(g.clone()));
    g
});

// Cardinality guard: limit the number of distinct error codes to avoid metrics explosion.
const MAX_ERROR_CODES: usize = 40; // tunable threshold
static ERROR_CODE_COUNT: AtomicUsize = AtomicUsize::new(0);
static OBSERVED_CODES: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));
const OVERFLOW_CODE: &str = "_overflow"; // label used when limit exceeded

/// Returns an Axum middleware function that records HTTP error counts.
/// Usage: .layer(axum::middleware::from_fn(http_error_metrics_layer("service-name")))
type HttpErrFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Result<axum::response::Response, ApiError>> + Send>>;
pub fn http_error_metrics_layer(service_name: &'static str) -> impl Fn(Request<Body>, Next) -> HttpErrFuture + Clone + Send + Sync + 'static {
    move |req: Request<Body>, next: Next| {
        let svc = service_name;
        Box::pin(async move {
            let resp = next.run(req).await;
            let status = resp.status();
            if status.as_u16() >= 400 {
                let raw_code = resp.headers().get("X-Error-Code").and_then(|v| v.to_str().ok()).unwrap_or("unknown");
                let code = if raw_code == OVERFLOW_CODE { OVERFLOW_CODE } else {
                    // Track distinct codes under guard
                    let mut set = OBSERVED_CODES.lock().expect("lock observed codes");
                    if set.contains(raw_code) {
                        raw_code
                    } else if ERROR_CODE_COUNT.load(Ordering::Relaxed) < MAX_ERROR_CODES {
                        set.insert(raw_code.to_string());
                        let new = ERROR_CODE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                        HTTP_ERROR_CODES_DISTINCT.set(new as i64);
                        HTTP_ERROR_CODE_SATURATION.set(((new as f64 / MAX_ERROR_CODES as f64) * 100.0).round() as i64);
                        raw_code
                    } else {
                        HTTP_ERROR_CODE_OVERFLOW_TOTAL.inc();
                        OVERFLOW_CODE
                    }
                };
                HTTP_ERRORS_TOTAL.with_label_values(&[svc, code, status.as_str()]).inc();
            }
            Ok(resp)
        })
    }
}

pub mod test_helpers {
    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use std::sync::MutexGuard;

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

    /// Test-only: simulate recording an error code just like the middleware would (without building an HTTP response)
    pub fn simulate_error_code(code: &str) {
        // Mirror guard logic
        let mut set: MutexGuard<HashSet<String>> = OBSERVED_CODES.lock().expect("lock observed codes");
        if set.contains(code) {
            return; // duplicate ignored for distinct tracking
        }
        if ERROR_CODE_COUNT.load(Ordering::Relaxed) < MAX_ERROR_CODES {
            set.insert(code.to_string());
            let new = ERROR_CODE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            HTTP_ERROR_CODES_DISTINCT.set(new as i64);
        } else {
            HTTP_ERROR_CODE_OVERFLOW_TOTAL.inc();
        }
    }

    pub fn distinct_gauge() -> i64 { HTTP_ERROR_CODES_DISTINCT.get() }
    pub fn overflow_count() -> u64 { HTTP_ERROR_CODE_OVERFLOW_TOTAL.get() }
    pub fn saturation_percent() -> i64 { HTTP_ERROR_CODE_SATURATION.get() }
}

/// Test-only assertion macro for validating an ApiError's rendered response structure.
/// Usage:
/// assert_api_error!(err, "missing_role");
#[macro_export]
macro_rules! assert_api_error {
    ($err:expr, $code:expr) => {{
        let err: $crate::ApiError = $err; // type ascription if inference ambiguous
        $crate::test_helpers::assert_error_shape(err, $code).await;
    }};
}
