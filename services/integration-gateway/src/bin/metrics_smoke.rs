//! Simple binary to print current metrics exposition to stdout.
//! Used in CI to assert presence of required gauges/counters.

use integration_gateway::metrics::GatewayMetrics;
use std::sync::Arc;
use common_security::Capability;
use uuid::Uuid;
use prometheus::Encoder; // trait for TextEncoder::encode

fn main() {
    // Initialize metrics as done in main (subset only). We don't need DB / rate limiter.
    let metrics = Arc::new(GatewayMetrics::new().expect("init metrics"));
    // Set target to deterministic test value for visibility
    metrics.set_rate_limit_rpm_target(1234);
    let resp = metrics.render().expect("render metrics");

    // Extract body bytes (axum Response into hyper Body, here already built in render)
    // render() returns Response<Body> so we need to block to collect (simpler: reconstruct via registry directly?)
    // For simplicity, use prometheus default registry gather like tests do.
    let registry = prometheus::default_registry();
    let mfs = registry.gather();
    let mut buf = Vec::new();
    let enc = prometheus::TextEncoder::new();
    enc.encode(&mfs, &mut buf).expect("encode");
    let text = String::from_utf8(buf).expect("utf8");
    // Touch capability enum so policy module (and its metric registrations) gets linked.
    let _ = Capability::InventoryView; // referencing ensures the object file is included
    println!("{}", text);

    // Basic presence checks (exit non-zero if missing).
    for required in ["gateway_rate_limit_rpm_target", "capability_checks_total", "gateway_build_info"] {
        if !text.contains(required) { eprintln!("missing required metric: {required}"); std::process::exit(1); }
    }
}
