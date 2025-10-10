use axum::{Router, routing::post, routing::get, http::Request};
use payment_service::{AppState, payment_handlers::{create_intent, confirm_intent, get_intent}};
use common_auth::{JwtVerifier, JwtConfig};
use std::sync::Arc;
use tower::ServiceExt;
use serde_json::json;

fn app() -> Router {
    let verifier = Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud")));
    let state = AppState { jwt_verifier: verifier, db: None, #[cfg(feature="kafka")] audit_producer: None };
    Router::new()
        .route("/payment_intents", post(create_intent))
        .route("/payment_intents/:id", get(get_intent))
        .route("/payment_intents/confirm", post(confirm_intent))
        .with_state(state)
}

#[tokio::test]
async fn create_and_confirm_intent_without_db() {
    let app = app();
    let body = json!({
        "id": "pi_test_123",
        "orderId": "ord_abc",
        "amountMinor": 1234,
        "currency": "USD",
        "idempotencyKey": "idem_1"
    }).to_string();
    let mut req = Request::builder().uri("/payment_intents").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    // Allow via capability by setting cashier role and tenant id
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", "00000000-0000-0000-0000-000000000000".parse().unwrap());
    headers.insert("X-Roles", "cashier".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success(), "status={}", resp.status());
    let bytes = axum::body::to_bytes(resp.into_body(), 1024*16).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["id"], "pi_test_123");
    assert_eq!(v["state"], "created");

    // Confirm
    let body = json!({"id": "pi_test_123"}).to_string();
    let mut req = Request::builder().uri("/payment_intents/confirm").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", "00000000-0000-0000-0000-000000000000".parse().unwrap());
    headers.insert("X-Roles", "cashier".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
    let bytes = axum::body::to_bytes(resp.into_body(), 1024*16).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["id"], "pi_test_123");
    assert_eq!(v["state"], "authorized");

    // Get (without DB, returns fallback)
    let mut req = Request::builder().uri("/payment_intents/pi_test_123").method("GET")
        .body(axum::body::Body::empty()).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", "00000000-0000-0000-0000-000000000000".parse().unwrap());
    headers.insert("X-Roles", "cashier".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
}
