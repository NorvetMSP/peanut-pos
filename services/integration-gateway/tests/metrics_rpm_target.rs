use integration_gateway::metrics::GatewayMetrics;

// Minimal stub to expose metrics endpoint similar to main without spinning full state.
#[tokio::test]
async fn metrics_exports_rpm_target_gauge() {
    // The real application registers metrics in global default registry; here we rely on side-effect
    // from importing integration_gateway::integration_handlers::metrics (assuming already in crate root init)
    // If the metric isn't registered test will fail string search.

    let gm = GatewayMetrics::new().expect("init metrics");
    gm.set_rate_limit_rpm_target(777);
    let body = gm.gather_text().expect("gather text");
    assert!(body.contains("gateway_rate_limit_rpm_target"), "expected gateway_rate_limit_rpm_target gauge in metrics scrape; got snippet: {}", &body[0..std::cmp::min(500, body.len())]);
}
