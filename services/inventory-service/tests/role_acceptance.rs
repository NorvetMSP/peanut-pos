//! Capability deny/allow matrix tests for InventoryView (TA-OPS-7 seed).
//! Verifies Support is denied while Cashier, Admin, SuperAdmin are allowed.

use axum::{Router, routing::get, http::{Request, StatusCode, HeaderValue}};
use tower::ServiceExt;
use crate::test_utils::lazy_app_state;
use common_security::{SecurityCtxExtractor, ensure_capability, Capability};

// bring in test utils module
mod test_utils;

use axum::response::IntoResponse;
async fn inventory_stub(SecurityCtxExtractor(sec): SecurityCtxExtractor) -> axum::response::Response {
	if ensure_capability(&sec, Capability::InventoryView).is_ok() {
		StatusCode::OK.into_response()
	} else {
		let mut resp = StatusCode::FORBIDDEN.into_response();
		resp.headers_mut().insert("X-Error-Code", HeaderValue::from_static("missing_role"));
		resp
	}
}

fn build_app() -> Router {
	Router::new().route("/inventory", get(inventory_stub)).with_state(lazy_app_state())
}

async fn req_with(role: &str) -> (StatusCode, axum::http::HeaderMap) {
	let app = build_app();
	let mut req = Request::builder().uri("/inventory").method("GET").body(axum::body::Body::empty()).unwrap();
	let h = req.headers_mut();
	h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
	h.insert("X-Roles", HeaderValue::from_str(role).unwrap());
	h.insert("X-User-ID", HeaderValue::from_static("22222222-2222-2222-2222-222222222222"));
	let resp = app.oneshot(req).await.unwrap();
	(resp.status(), resp.headers().clone())
}

#[tokio::test]
async fn support_denied_inventory_view() {
    let (status, headers) = req_with("support").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(headers.get("X-Error-Code").and_then(|v| v.to_str().ok()), Some("missing_role"));
}

#[tokio::test]
async fn cashier_allowed_inventory_view() {
	let (status, _headers) = req_with("cashier").await;
	assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn admin_allowed_inventory_view() {
	let (status, _headers) = req_with("admin").await;
	assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn superadmin_allowed_inventory_view() {
	let (status, _headers) = req_with("superadmin").await;
	assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn manager_allowed_inventory_view() {
	let (status, _headers) = req_with("manager").await;
	assert_eq!(status, StatusCode::OK);
}