use axum::{Router, routing::get, http::{Request, StatusCode}};
use common_security::SecurityCtxExtractor;
use tower::ServiceExt; // oneshot

// Synthetic handler that requires extractor and returns OK if headers present
async fn ok(SecurityCtxExtractor(_sec): SecurityCtxExtractor) -> &'static str { "ok" }

#[tokio::test]
async fn missing_tenant_400() {
    // Route expects extractor; absence of X-Tenant-ID should cause 400 with header
    let app = Router::new().route("/ping", get(ok));
    let req = Request::builder().uri("/ping").method("GET").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_tenant_id");
}

#[tokio::test]
async fn internal_error_500() {
    use axum::routing::get;
    use common_http_errors::ApiError;
    async fn boom() -> Result<String, ApiError> { Err(ApiError::Internal { trace_id: None, message: Some("synthetic".into()) }) }
    let app = Router::new().route("/boom", get(boom));
    let req = Request::builder().uri("/boom").method("GET").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "internal_error");
}
