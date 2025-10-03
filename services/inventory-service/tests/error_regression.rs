use axum::{Router, routing::get, http::{Request, StatusCode, HeaderValue}};
use inventory_service::inventory_handlers::list_inventory;
use tower::ServiceExt;
use crate::test_utils::lazy_app_state;

mod test_utils;


#[tokio::test]
async fn missing_tenant_returns_400() {
    let state = lazy_app_state();
    let app = Router::new().route("/inventory", get(list_inventory)).with_state(state);
    let req = Request::builder().uri("/inventory").method("GET").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_tenant_id");
}

#[tokio::test]
async fn forbidden_role_returns_403() {
    let state = lazy_app_state();
    let app = Router::new().route("/inventory", get(list_inventory)).with_state(state);
    let mut req = Request::builder().uri("/inventory").method("GET").body(axum::body::Body::empty()).unwrap();
    {
        let h = req.headers_mut();
        h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
        h.insert("X-Roles", HeaderValue::from_static("support"));
        h.insert("X-User-ID", HeaderValue::from_static("22222222-2222-2222-2222-222222222222"));
    }
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
}

#[tokio::test]
async fn internal_error_500() {
    use axum::routing::get;
    use common_http_errors::ApiError;
    async fn boom() -> Result<String, ApiError> { Err(ApiError::Internal { trace_id: None, message: Some("synthetic".into()) }) }
    // No state required for this synthetic endpoint
    let app = Router::new().route("/boom", get(boom));
    let req = Request::builder().uri("/boom").method("GET").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "internal_error");
}
