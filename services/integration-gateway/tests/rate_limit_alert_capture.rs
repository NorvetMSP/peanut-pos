//! Integration test asserting rate limit alert event capture via TEST_CAPTURE_KAFKA harness.
//! This mirrors the payment void capture pattern and ensures alert publisher participates
//! in the unified capture mechanism.


use once_cell::sync::Lazy;

static INIT: Lazy<()> = Lazy::new(|| {
    // Ensure env vars only set once for this test process
    std::env::set_var("TEST_CAPTURE_KAFKA", "1");
    std::env::set_var("TEST_KAFKA_NO_BROKER", "1");
});

#[tokio::test]
async fn rate_limit_alert_event_is_captured() {
    Lazy::force(&INIT);

    // Simulate calling the alert publisher directly (unit-style integration) because
    // constructing full gateway state & generating natural traffic would be heavier.
    // We only need to prove the capture hook fires.
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    {
        use integration_gateway::alerts::{publish_rate_limit_alert, RateLimitAlertEvent};
        use rdkafka::producer::FutureProducer;

        // Build a dummy producer; it will not actually connect due to TEST_KAFKA_NO_BROKER=1
        let producer: FutureProducer = rdkafka::config::ClientConfig::new()
            .set("bootstrap.servers", "localhost:9092")
            .create()
            .expect("producer");

        use chrono::Utc;
        let event = RateLimitAlertEvent {
            action: "rate_limit.exceeded",
            tenant_id: None,
            key_hash: Some("abc123".into()),
            key_suffix: Some("/v1/payments".into()),
            identity: "anon".into(),
            limit: 10,
            count: 11,
            window_seconds: 60,
            occurred_at: Utc::now(),
            message: "Exceeded payment route limit".into(),
        };

        publish_rate_limit_alert(&producer, "rate_limit_alerts", &event)
            .await
            .expect("publish");

        // Reuse existing payment void capture vector for simplicity; ensure at least one entry
        let captured = integration_gateway::integration_handlers::test_support::take_captured_rate_limit_alerts();
        assert!(!captured.is_empty(), "expected at least one captured alert event payload (rate limit alert capture vector)");
        let payload = &captured[0];
        assert!(payload.contains("rate_limit.exceeded"), "payload missing action field: {payload}");
        assert!(payload.contains("\"count\":11"), "payload missing count field: {payload}");
    }
}
