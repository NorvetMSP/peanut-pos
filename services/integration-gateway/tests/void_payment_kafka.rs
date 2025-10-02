//! Kafka feature integration test asserting `payment.voided` event capture.
#![cfg(any(feature = "kafka", feature = "kafka-producer"))]

use axum::{Router, routing::post, body::Body, http::Request};
use tower::ServiceExt;
use uuid::Uuid;
use serde_json::json;
use integration_gateway::integration_handlers::{void_payment, test_support};
use integration_gateway::{AppState, GatewayConfig, GatewayMetrics, UsageTracker};
use std::sync::Arc;
use common_auth::{JwtConfig, JwtVerifier};

async fn build_state() -> AppState {
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
    }));
    let metrics = Arc::new(GatewayMetrics::new().unwrap());
    let pool = sqlx::postgres::PgPoolOptions::new().max_connections(1).connect_lazy("postgres://postgres:postgres@localhost:5432/postgres").unwrap();
    let producer: rdkafka::producer::FutureProducer = rdkafka::ClientConfig::new().set("bootstrap.servers","localhost:9092").create().expect("producer");
    let usage = UsageTracker::new(config.clone(), pool.clone(), Some(producer.clone()));
    let jwt_verifier = Arc::new(futures::executor::block_on(async { JwtVerifier::builder(JwtConfig::new("issuer","aud")).build().await.unwrap() }));
    // Build test state using in-memory rate limiter helper (window 60s)
    let mut state = AppState::test_with_in_memory(60, config.clone(), metrics.clone(), usage, jwt_verifier);
    // Overwrite auto-created producer with our configured one for deterministic keying
    state.kafka_producer = producer;
    state
}

#[tokio::test]
async fn void_payment_emits_captured_event() {
    std::env::set_var("TEST_CAPTURE_KAFKA","1");
    std::env::set_var("TEST_KAFKA_NO_BROKER","1");
    // Drain any prior captured events
    #[cfg(test)] let _ = test_support::take_captured_payment_voided();
    let state = build_state().await;
    let app = Router::new().route("/payments/void", post(void_payment)).with_state(state);
    let tenant = Uuid::new_v4();
    let order = Uuid::new_v4();
    let body = json!({"orderId": order.to_string(), "method": "card", "amount": 10.0, "reason": "test"});
    let mut req = Request::builder().uri("/payments/void").method("POST").header("Content-Type","application/json").body(Body::from(body.to_string())).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", tenant.to_string().parse().unwrap());
    h.insert("X-Roles", "support".parse().unwrap());
    h.insert("X-User-ID", tenant.to_string().parse().unwrap());
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_success(), "handler did not return success");
    #[cfg(test)] let captured = test_support::take_captured_payment_voided();
    #[cfg(test)] {
    assert_eq!(captured.len(), 1, "expected exactly one captured event, got {:?}", captured);
    let payload = &captured[0];
    assert!(payload.contains(&order.to_string()), "payload missing order id: {payload}");
    assert!(payload.contains("order_id") && payload.contains("tenant_id") && payload.contains("reason"), "payload missing expected keys: {payload}");
    }
}
