//! Deny-path test for capability enforcement: Support should be denied PaymentProcess, Cashier allowed.
#![cfg(any(feature = "kafka", feature = "kafka-producer"))]

use axum::{Router, routing::post, http::{Request, StatusCode}};
use tower::ServiceExt;
use integration_gateway::integration_handlers::process_payment;
use integration_gateway::{AppState, GatewayConfig, GatewayMetrics, UsageTracker};
use common_auth::{JwtConfig, JwtVerifier};
use std::sync::Arc;
use serde_json::json;

async fn state() -> AppState {
    let config = Arc::new(GatewayConfig::from_env().unwrap_or_else(|_| GatewayConfig {
        redis_url: "ignored".into(),
        redis_prefix: "itg_test".into(),
        rate_limit_rpm: 1000,
        rate_limit_window_secs: 60,
        rate_limit_burst_multiplier: 2.0,
        rate_limit_alert_cooldown_secs: 300,
        audit_topic: "audit.events.v1".into(),
        alert_topic: "security.alerts.v1".into(),
        api_usage_flush_secs: 60,
        api_usage_summary_secs: 300,
        security_alert_webhook_url: None,
        security_alert_webhook_bearer: None,
        payment_service_fallback_auth: None,
    }));
    let metrics = Arc::new(GatewayMetrics::new().unwrap());
    let pool = sqlx::postgres::PgPoolOptions::new().max_connections(1).connect_lazy("postgres://postgres:postgres@localhost:5432/postgres").unwrap();
    let producer: rdkafka::producer::FutureProducer = rdkafka::ClientConfig::new().set("bootstrap.servers","localhost:9092").create().expect("producer");
    let usage = UsageTracker::new(config.clone(), pool.clone(), Some(producer.clone()));
    let jwt = Arc::new(futures::executor::block_on(async { JwtVerifier::builder(JwtConfig::new("issuer","aud")).build().await.unwrap() }));
    let mut s = AppState::test_with_in_memory(60, config, metrics, usage, jwt);
    s.kafka_producer = producer;
    s
}

fn payment_request_body() -> String {
    json!({"orderId": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "method": "card", "amount": 12.34}).to_string()
}

#[tokio::test]
async fn support_role_denied_payment_process() {
    let app = Router::new().route("/payments", post(process_payment)).with_state(state().await);
    let mut req = Request::builder().uri("/payments").method("POST").header("content-type","application/json").body(axum::body::Body::from(payment_request_body())).unwrap();
    // Synthesize headers (middleware normally does this)
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", "11111111-1111-1111-1111-111111111111".parse().unwrap());
    h.insert("X-Roles", "support".parse().unwrap());
    h.insert("X-User-ID", "22222222-2222-2222-2222-222222222222".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
}

#[tokio::test]
async fn cashier_role_allowed_payment_process() {
    let app = Router::new().route("/payments", post(process_payment)).with_state(state().await);
    let mut req = Request::builder().uri("/payments").method("POST").header("content-type","application/json").body(axum::body::Body::from(payment_request_body())).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", "11111111-1111-1111-1111-111111111111".parse().unwrap());
    h.insert("X-Roles", "cashier".parse().unwrap());
    h.insert("X-User-ID", "22222222-2222-2222-2222-222222222222".parse().unwrap());
    let resp = app.oneshot(req).await.unwrap();
    // Upstream payment-service call will fail (no service running) -> expect internal or declined.
    // We only validate it is NOT a forbidden missing_role.
    assert_ne!(resp.status(), StatusCode::FORBIDDEN, "cashier should not be forbidden for payment_process capability");
}
