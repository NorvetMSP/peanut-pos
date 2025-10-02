use axum::{Router, routing::{get, post}};
use common_security::{SecurityCtxExtractor, Capability, ensure_capability};
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

// Capability-only protected stub (no legacy role fallback)
mod payment_capability_tests {
    use super::*;
    use common_http_errors::ApiError;

    async fn payment_stub(SecurityCtxExtractor(sec): SecurityCtxExtractor) -> Result<&'static str, ApiError> {
        ensure_capability(&sec, Capability::PaymentProcess)
            .map_err(|_| ApiError::ForbiddenMissingRole { role: "payment_process", trace_id: sec.trace_id })?;
        Ok("processed")
    }

    #[tokio::test]
    async fn support_role_denied() {
        let app = Router::new().route("/pay", post(payment_stub));
        let mut req = Request::builder().uri("/pay").method("POST").body(axum::body::Body::empty()).unwrap();
        let h = req.headers_mut();
        h.insert("X-Tenant-ID", Uuid::new_v4().to_string().parse().unwrap());
        h.insert("X-Roles", "support".parse().unwrap());
        h.insert("X-User-ID", Uuid::new_v4().to_string().parse().unwrap());
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
        assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
    }

    #[tokio::test]
    async fn cashier_role_allowed() {
        let app = Router::new().route("/pay", post(payment_stub));
        let mut req = Request::builder().uri("/pay").method("POST").body(axum::body::Body::empty()).unwrap();
        let h = req.headers_mut();
        h.insert("X-Tenant-ID", Uuid::new_v4().to_string().parse().unwrap());
        h.insert("X-Roles", "cashier".parse().unwrap());
        h.insert("X-User-ID", Uuid::new_v4().to_string().parse().unwrap());
        let resp = app.oneshot(req).await.unwrap();
        assert!(resp.status().is_success(), "expected success, got {}", resp.status());
    }
}
