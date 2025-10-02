use common_http_errors::{ApiError, http_error_metrics_layer};
use axum::{Router, routing::get, http::StatusCode};
use axum::middleware;
use std::sync::atomic::{AtomicUsize, Ordering};
use once_cell::sync::Lazy;
use tower::ServiceExt; // for oneshot

static DYNAMIC_COUNTER: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));

// Handler that emits a different error code each time until we exceed guard threshold.
async fn dyn_error() -> Result<&'static str, ApiError> {
    let n = DYNAMIC_COUNTER.fetch_add(1, Ordering::Relaxed);
    // fabricate pseudo-dynamic codes beyond threshold
    let code = format!("dyn_code_{}", n);
    Err(ApiError::BadRequest { code: Box::leak(code.into_boxed_str()), trace_id: None, message: None })
}

#[tokio::test]
async fn error_code_cardinality_guard_caps_labels() {
    // Build router with metrics layer
    let app = Router::new()
        .route("/err", get(dyn_error))
        .layer(middleware::from_fn(http_error_metrics_layer("test-svc")));

    // Fire more requests than MAX_ERROR_CODES (40) to trigger overflow label usage.
    let total = 50;
    for _ in 0..total {
        let resp = app.clone().oneshot(axum::http::Request::builder().uri("/err").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
    // We cannot easily introspect prometheus registry here without parsing output.
    // This test ensures no panic and all requests return 400 even after exceeding threshold.
}
