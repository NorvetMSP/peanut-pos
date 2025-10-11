use axum::{Router, routing::post, http::Request};
use axum::response::IntoResponse;
use tower::ServiceExt;
use payment_service::{AppState, webhook::verify_webhook};
use common_auth::{JwtVerifier, JwtConfig};
use std::sync::Arc;
use hmac::{Hmac, Mac};
use sha2::{Sha256, Digest};

fn test_router() -> Router {
    let verifier = Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud")));
    let state = AppState { jwt_verifier: verifier, db: None, #[cfg(feature="kafka")] audit_producer: None };
    async fn ok_handler(body: String) -> impl IntoResponse { (axum::http::StatusCode::OK, body) }
    Router::new()
        .route("/webhooks/test", post(ok_handler))
        .layer(axum::middleware::from_fn(verify_webhook))
        .with_state(state)
}

fn sign(secret: &str, ts: &str, nonce: &str, body: &[u8]) -> String {
    let body_hash = format!("{:x}", sha2::Sha256::digest(body));
    let canonical = format!("ts:{}\nnonce:{}\nbody_sha256:{}", ts, nonce, body_hash);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(canonical.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());
    format!("sha256={}", expected)
}

#[tokio::test]
async fn webhook_happy_path_ok() {
    std::env::set_var("WEBHOOK_ACTIVE_SECRET", "s3cr3t");
    std::env::remove_var("WEBHOOK_MAX_SKEW_SECS");
    let app = test_router();
    let body = b"{\"ok\":true}".to_vec();
    let ts = format!("{}", chrono::Utc::now().timestamp());
    let nonce = "nonce-1";
    let sig = sign("s3cr3t", &ts, nonce, &body);
    let req = Request::builder()
        .uri("/webhooks/test")
        .method("POST")
        .header("content-type", "application/json")
        .header("X-Timestamp", ts)
        .header("X-Nonce", nonce)
        .header("X-Signature", sig)
        .body(axum::body::Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
}

#[tokio::test]
async fn webhook_signature_mismatch_401() {
    std::env::set_var("WEBHOOK_ACTIVE_SECRET", "s3cr3t");
    let app = test_router();
    let body = b"{}".to_vec();
    let ts = format!("{}", chrono::Utc::now().timestamp());
    let nonce = "nonce-2";
    // Wrong secret
    let sig = sign("wrong", &ts, nonce, &body);
    let req = Request::builder()
        .uri("/webhooks/test")
        .method("POST")
        .header("content-type", "application/json")
        .header("X-Timestamp", ts)
        .header("X-Nonce", nonce)
        .header("X-Signature", sig)
        .body(axum::body::Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 401);
    let code = resp.headers().get("X-Error-Code").and_then(|v| v.to_str().ok()).unwrap_or("");
    assert_eq!(code, "sig_mismatch");
}

#[tokio::test]
async fn webhook_timestamp_skew_401() {
    std::env::set_var("WEBHOOK_ACTIVE_SECRET", "s3cr3t");
    std::env::set_var("WEBHOOK_MAX_SKEW_SECS", "1");
    let app = test_router();
    let body = b"{}".to_vec();
    // Too old by 5 seconds
    let ts = format!("{}", chrono::Utc::now().timestamp() - 5);
    let nonce = "nonce-3";
    let sig = sign("s3cr3t", &ts, nonce, &body);
    let req = Request::builder()
        .uri("/webhooks/test")
        .method("POST")
        .header("content-type", "application/json")
        .header("X-Timestamp", ts)
        .header("X-Nonce", nonce)
        .header("X-Signature", sig)
        .body(axum::body::Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 401);
    let code = resp.headers().get("X-Error-Code").and_then(|v| v.to_str().ok()).unwrap_or("");
    assert_eq!(code, "sig_skew");
}

async fn app_with_db(db: sqlx::PgPool) -> Router {
    let verifier = Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud")));
    let state = AppState { jwt_verifier: verifier, db: Some(db), #[cfg(feature="kafka")] audit_producer: None };
    async fn ok_handler(body: String) -> impl IntoResponse { (axum::http::StatusCode::OK, body) }
    Router::new()
        .route("/webhooks/test", post(ok_handler))
        .layer(axum::middleware::from_fn(verify_webhook))
        .with_state(state)
}

#[tokio::test]
#[ignore]
async fn webhook_nonce_replay_db_401() {
    use sqlx::{PgPool, Executor};
    std::env::set_var("WEBHOOK_ACTIVE_SECRET", "s3cr3t");
    let dsn = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for this ignored test");
    let pool = PgPool::connect(&dsn).await.unwrap();
    // Ensure table exists for local runs
    pool.execute(r#"
    CREATE TABLE IF NOT EXISTS webhook_nonces (
        nonce TEXT PRIMARY KEY,
        ts    TIMESTAMPTZ NOT NULL DEFAULT now(),
        provider TEXT
    );
    "#).await.unwrap();

    let app = app_with_db(pool).await;
    let body = b"{}".to_vec();
    let ts = format!("{}", chrono::Utc::now().timestamp());
    let nonce = "nonce-db-replay";
    let sig = sign("s3cr3t", &ts, nonce, &body);

    // First request succeeds
    let req1 = Request::builder()
        .uri("/webhooks/test")
        .method("POST")
        .header("content-type", "application/json")
        .header("X-Timestamp", ts.clone())
        .header("X-Nonce", nonce)
        .header("X-Signature", sig.clone())
        .header("X-Provider", "itest")
        .body(axum::body::Body::from(body.clone()))
        .unwrap();
    let resp1 = app.clone().oneshot(req1).await.unwrap();
    assert!(resp1.status().is_success());

    // Second request with same nonce → 401 replay
    let req2 = Request::builder()
        .uri("/webhooks/test")
        .method("POST")
        .header("content-type", "application/json")
        .header("X-Timestamp", ts)
        .header("X-Nonce", nonce)
        .header("X-Signature", sig)
        .header("X-Provider", "itest")
        .body(axum::body::Body::from(body))
        .unwrap();
    let resp2 = app.clone().oneshot(req2).await.unwrap();
    assert_eq!(resp2.status().as_u16(), 401);
    let code = resp2.headers().get("X-Error-Code").and_then(|v| v.to_str().ok()).unwrap_or("");
    assert_eq!(code, "sig_replay");
}
