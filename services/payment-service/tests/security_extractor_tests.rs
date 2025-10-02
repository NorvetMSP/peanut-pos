use axum::{Router, routing::post};
use axum::http::{Request, StatusCode, HeaderValue};
use payment_service::{AppState, payment_handlers::process_card_payment};
use std::sync::Arc;
use common_auth::{JwtVerifier, JwtConfig};
use tower::ServiceExt;
use serde_json::json;

// Build minimal app with process_card_payment route only
async fn app() -> Router {
    let verifier = Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud")));
    let state = AppState { jwt_verifier: verifier };
    Router::new()
        .route("/payments", post(process_card_payment))
        .with_state(state)
}

#[tokio::test]
async fn payment_missing_tenant_header() {
    let app = app().await;
    let body = json!({"orderId":"abc123","method":"card","amount":"10"}).to_string();
    let req = Request::builder().uri("/payments").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn payment_forbidden_role() {
    let app = app().await;
    let body = json!({"orderId":"abc123","method":"card","amount":"10"}).to_string();
    let mut req = Request::builder().uri("/payments").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", HeaderValue::from_static("00000000-0000-0000-0000-000000000000"));
    headers.insert("X-Roles", HeaderValue::from_static("support")); // not in PAYMENT_ROLES mapping
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
