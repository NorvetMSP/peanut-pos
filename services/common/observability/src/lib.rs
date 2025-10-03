use prometheus::{IntCounter, Histogram, Registry, IntCounterVec};

#[derive(Clone)]
pub struct InventoryMetrics {
    pub registry: Registry,
    pub dual_write_divergence: IntCounter,
    pub reservation_expired: IntCounter,
    pub audit_emit_failures: IntCounter,
    pub sweeper_duration_seconds: Histogram,
    pub heal_latency_seconds: Histogram,
    pub http_errors_total: IntCounterVec,
}

impl InventoryMetrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        let dual_write_divergence = IntCounter::new(
            "dual_write_divergence_total",
            "Dual write divergence occurrences",
        ).unwrap();
        let reservation_expired = IntCounter::new(
            "inventory_reservation_expired_total",
            "Expired reservations count",
        ).unwrap();
        let audit_emit_failures = IntCounter::new(
            "audit_event_emit_failures_total",
            "Audit event emission failures",
        ).unwrap();
        let sweeper_duration_seconds = Histogram::with_opts(
            prometheus::HistogramOpts::new(
                "inventory_reservation_sweeper_duration_seconds",
                "Duration of a reservation expiration sweep"
            ).buckets(vec![0.01,0.05,0.1,0.25,0.5,1.0,2.0,5.0])
        ).unwrap();
        let heal_latency_seconds = Histogram::with_opts(
            prometheus::HistogramOpts::new(
                "dual_write_heal_latency_seconds",
                "Time spent attempting to heal a dual-write divergence"
            ).buckets(vec![0.001,0.005,0.01,0.05,0.1,0.25,0.5])
        ).unwrap();
        let http_errors_total = IntCounterVec::new(
            prometheus::Opts::new(
                "http_errors_total",
                "Count of HTTP error responses emitted (status >= 400)"
            ),
            &["service", "code", "status"]
        ).unwrap();
        let _ = registry.register(Box::new(dual_write_divergence.clone()));
        let _ = registry.register(Box::new(reservation_expired.clone()));
        let _ = registry.register(Box::new(audit_emit_failures.clone()));
        let _ = registry.register(Box::new(sweeper_duration_seconds.clone()));
        let _ = registry.register(Box::new(heal_latency_seconds.clone()));
        let _ = registry.register(Box::new(http_errors_total.clone()));
        InventoryMetrics { registry, dual_write_divergence, reservation_expired, audit_emit_failures, sweeper_duration_seconds, heal_latency_seconds, http_errors_total }
    }
}

impl Default for InventoryMetrics {
    fn default() -> Self { Self::new() }
}
