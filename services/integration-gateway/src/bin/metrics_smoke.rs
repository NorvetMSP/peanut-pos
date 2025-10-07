//! Simple binary to print current metrics exposition to stdout.
//! Used in CI to assert presence of required gauges/counters.

use integration_gateway::metrics::GatewayMetrics;
use std::sync::Arc;
use common_security::{Capability, ensure_capability, SecurityContext, Role};
use common_audit::AuditActor;
use uuid::Uuid;
use prometheus::Encoder; // trait for TextEncoder::encode

fn main() {
    // Initialize metrics as done in main (subset only). We don't need DB / rate limiter.
    let metrics = Arc::new(GatewayMetrics::new().expect("init metrics"));
    // Set deterministic values for visibility
    metrics.set_rate_limit_rpm_target(1234);
    metrics.set_build_info();

    // Touch capability metrics by performing one allow and one deny check to ensure time series exist.
    let dummy_ctx = SecurityContext {
        tenant_id: Uuid::new_v4(),
        actor: AuditActor { id: Some(Uuid::new_v4()), name: None, email: None },
        roles: vec![Role::Cashier],
        trace_id: None,
    };
    // Cashier: allowed for PaymentProcess, denied for CustomerWrite (by design)
    let _ = ensure_capability(&dummy_ctx, Capability::PaymentProcess);
    let _ = ensure_capability(&dummy_ctx, Capability::CustomerWrite);

    // Gather both the gateway registry and the default registry (used by common-security capability metrics)
    // First, text from gateway's own registry
    let mut text = metrics.gather_text().expect("gather gateway metrics");
    // Then, append default registry exposition
    let registry = prometheus::default_registry();
    let mfs = registry.gather();
    let mut buf = Vec::new();
    let enc = prometheus::TextEncoder::new();
    enc.encode(&mfs, &mut buf).expect("encode");
    text.push_str(&String::from_utf8(buf).expect("utf8"));
    // Touch capability enum so policy module (and its metric registrations) gets linked.
    let _ = Capability::InventoryView; // referencing ensures the object file is included
    println!("{}", text);

    // Basic presence checks (exit non-zero if missing).
    for required in ["gateway_rate_limit_rpm_target", "capability_checks_total", "gateway_build_info"] {
        if !text.contains(required) { eprintln!("missing required metric: {required}"); std::process::exit(1); }
    }
}
