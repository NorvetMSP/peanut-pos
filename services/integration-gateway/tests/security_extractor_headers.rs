use axum::{Router, routing::get};
use common_security::SecurityCtxExtractor;
use axum::http::Request;
use tower::util::ServiceExt; // for oneshot
use uuid::Uuid;

// Because auth_middleware is complex (kafka, db, etc.), we simulate synthesized headers directly.
#[tokio::test]
async fn synthesized_headers_allow_extractor() {
    // Minimal stub router: we don't hit real payment logic (body validation would fail before DB if missing fields)
    async fn ok_handler(SecurityCtxExtractor(_sec): SecurityCtxExtractor) -> &'static str { "ok" }
    let app = Router::new().route("/ping", get(ok_handler));

    let tenant = Uuid::new_v4();
    let user = Uuid::new_v4();
    let mut req = Request::builder().uri("/ping").method("GET").body(axum::body::Body::empty()).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", tenant.to_string().parse().unwrap());
    headers.insert("X-Roles", "support".parse().unwrap());
    headers.insert("X-User-ID", user.to_string().parse().unwrap());

    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success(), "expected 2xx, got {}", resp.status());
}
