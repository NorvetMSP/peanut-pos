use payment_service::PAYMENT_ROLES;
use common_security::{roles::{ensure_any_role, Role}, SecurityCtxExtractor};
use common_security::context::SecurityContext;
use uuid::Uuid;
use common_audit::AuditActor;
use axum::{Router, routing::post};
use payment_service::{payment_handlers::process_card_payment, AppState};
use common_auth::{JwtVerifier, JwtConfig};
use std::sync::Arc;
use axum::body::Body;
use axum::http::{Request, StatusCode, HeaderValue};
use tower::ServiceExt;
use serde_json::json;

fn mk_ctx(role: Role) -> SecurityContext {
    SecurityContext {
        tenant_id: Uuid::new_v4(),
        actor: AuditActor { id: Some(Uuid::new_v4()), name: None, email: None },
        roles: vec![role],
        trace_id: None,
    }
}

#[test]
fn cashier_role_allowed_for_payment() {
    let ctx = mk_ctx(Role::Cashier);
    ensure_any_role(&ctx, PAYMENT_ROLES).expect("cashier should be permitted in PAYMENT_ROLES");
}

#[test]
fn superadmin_role_allowed_for_payment() {
    let ctx = mk_ctx(Role::SuperAdmin);
    ensure_any_role(&ctx, PAYMENT_ROLES).expect("superadmin should be permitted in PAYMENT_ROLES");
}

fn test_app() -> Router {
    let state = AppState { jwt_verifier: Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud"))) };
    Router::new().route("/payments", post(process_card_payment)).with_state(state)
}

#[tokio::test]
async fn support_role_denied_payment_process() {
    let app = test_app();
    let body = json!({"orderId":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","method":"card","amount":"10.00"}).to_string();
    let mut req = Request::builder().uri("/payments").method("POST").header("content-type","application/json").body(Body::from(body)).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("support"));
    h.insert("X-User-ID", HeaderValue::from_static("22222222-2222-2222-2222-222222222222"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
}

#[tokio::test]
async fn cashier_role_allowed_payment_process() {
    let app = test_app();
    let body = json!({"orderId":"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb","method":"card","amount":"5.00"}).to_string();
    let mut req = Request::builder().uri("/payments").method("POST").header("content-type","application/json").body(Body::from(body)).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("cashier"));
    h.insert("X-User-ID", HeaderValue::from_static("33333333-3333-3333-3333-333333333333"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}