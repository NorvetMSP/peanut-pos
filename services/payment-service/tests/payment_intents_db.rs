use axum::{Router, routing::{post, get}, http::Request};
use payment_service::{AppState, payment_handlers::{create_intent, confirm_intent, capture_intent, refund_intent, void_intent, get_intent}};
use common_auth::{JwtVerifier, JwtConfig};
use std::sync::Arc;
use tower::ServiceExt;
use serde_json::json;
use sqlx::{PgPool, Executor};

async fn app_with_db(db: PgPool) -> Router {
    let verifier = Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud")));
    let state = AppState { jwt_verifier: verifier, db: Some(db), #[cfg(feature="kafka")] audit_producer: None };
    Router::new()
        .route("/payment_intents", post(create_intent))
        .route("/payment_intents/:id", get(get_intent))
        .route("/payment_intents/confirm", post(confirm_intent))
        .route("/payment_intents/capture", post(capture_intent))
        .route("/payment_intents/refund", post(refund_intent))
        .route("/payment_intents/void", post(void_intent))
        .with_state(state)
}

#[tokio::test]
#[ignore]
async fn db_backed_transitions_and_conflicts() {
    let dsn = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for this ignored test");
    let pool = PgPool::connect(&dsn).await.unwrap();

    // Ensure migration exists (table present). This test assumes migrations have been applied.
    // As a safety, create table if missing minimally compatible with 8002.
    pool.execute(r#"
    CREATE TABLE IF NOT EXISTS payment_intents (
        id TEXT PRIMARY KEY,
        order_id TEXT NOT NULL,
        amount_minor BIGINT NOT NULL,
        currency TEXT NOT NULL,
        state TEXT NOT NULL,
        provider TEXT NULL,
        provider_ref TEXT NULL,
        idempotency_key TEXT NULL,
        metadata_json JSONB NULL,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
    );
    "#).await.unwrap();

    let app = app_with_db(pool).await;

    // Create intent
    let body = json!({
        "id": "pi_db_1",
        "orderId": "ord1",
        "amountMinor": 500,
        "currency": "USD",
        "idempotencyKey": "idem_db_1"
    }).to_string();
    let mut req = Request::builder().uri("/payment_intents").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", "00000000-0000-0000-0000-000000000000".parse().unwrap());
    headers.insert("X-Roles", "cashier".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success());

    // Invalid: capture before confirm -> expect 409
    let body = json!({"id":"pi_db_1"}).to_string();
    let mut req = Request::builder().uri("/payment_intents/capture").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", "00000000-0000-0000-0000-000000000000".parse().unwrap());
    headers.insert("X-Roles", "cashier".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 409);

    // Confirm (created -> authorized)
    let body = json!({"id":"pi_db_1"}).to_string();
    let mut req = Request::builder().uri("/payment_intents/confirm").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", "00000000-0000-0000-0000-000000000000".parse().unwrap());
    headers.insert("X-Roles", "cashier".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success());

    // Now capture is valid
    let body = json!({"id":"pi_db_1"}).to_string();
    let mut req = Request::builder().uri("/payment_intents/capture").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", "00000000-0000-0000-0000-000000000000".parse().unwrap());
    headers.insert("X-Roles", "cashier".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success());

    // Invalid: capture again after captured -> 409
    let body = json!({"id":"pi_db_1"}).to_string();
    let mut req = Request::builder().uri("/payment_intents/capture").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", "00000000-0000-0000-0000-000000000000".parse().unwrap());
    headers.insert("X-Roles", "cashier".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 409);

    // Refund from captured -> ok
    let body = json!({"id":"pi_db_1"}).to_string();
    let mut req = Request::builder().uri("/payment_intents/refund").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", "00000000-0000-0000-0000-000000000000".parse().unwrap());
    headers.insert("X-Roles", "cashier".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success());

    // Void after refunded -> 409
    let body = json!({"id":"pi_db_1"}).to_string();
    let mut req = Request::builder().uri("/payment_intents/void").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", "00000000-0000-0000-0000-000000000000".parse().unwrap());
    headers.insert("X-Roles", "cashier".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 409);
}
