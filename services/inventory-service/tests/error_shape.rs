use axum::response::IntoResponse; // for converting ApiError
use http_body_util::BodyExt; // for collect()
use inventory_service::list_inventory;
mod test_utils;
use test_utils::lazy_app_state;
use axum::{Router, routing::get};
use tower::ServiceExt; // for oneshot
use axum::http::Request;

#[tokio::test]
async fn list_inventory_missing_tenant_header_error_shape() {
    let state = lazy_app_state();

    // Call through router without X-Tenant-ID header so extractor rejects
    let app = Router::new().route("/inventory", get(list_inventory)).with_state(state);
    let req = Request::builder().uri("/inventory").method("GET").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    // Collect the body (hyper 1.0 pattern via http-body-util)
    let collected = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(collected.to_vec()).unwrap();
    // Extractor now returns structured ApiError JSON with code field.
    assert!(text.contains("\"code\":\"missing_tenant_id\""), "body was: {text}");
    // Header asserted indirectly by regression tests; just ensure JSON structure here.
}
