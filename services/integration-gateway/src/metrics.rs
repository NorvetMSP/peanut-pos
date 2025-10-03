use anyhow::Result;
use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use prometheus::{Encoder, IntCounterVec, Opts, Registry, TextEncoder, IntGauge, Histogram, HistogramOpts};
use std::env;

#[derive(Clone)]
pub struct GatewayMetrics {
    registry: Registry,
    rate_checks: IntCounterVec,
    rate_rejections: IntCounterVec,
    api_key_requests: IntCounterVec,
    // Backpressure / queue related gauges (TA-PERF-2)
    channel_depth: IntGauge,
    channel_capacity: IntGauge,
    channel_high_water: IntGauge,
    // Rate limiter metrics (TA-PERF-3)
    rate_limit_latency: Histogram,
    rate_window_usage: IntGauge,
    rate_limit_rpm_target: IntGauge,
    build_info: IntGauge,
}

impl GatewayMetrics {
    pub fn new() -> Result<Self> {
        let registry = Registry::new();
        let rate_checks = IntCounterVec::new(
            Opts::new("gateway_rate_limit_checks_total", "Total rate limit checks"),
            &["identity"],
        )?;
        let rate_rejections = IntCounterVec::new(
            Opts::new(
                "gateway_rate_limit_rejections_total",
                "Total rate limit rejections",
            ),
            &["identity"],
        )?;
        let api_key_requests = IntCounterVec::new(
            Opts::new(
                "gateway_api_key_requests_total",
                "API key requests grouped by result",
            ),
            &["result"],
        )?;
        registry.register(Box::new(rate_checks.clone()))?;
        registry.register(Box::new(rate_rejections.clone()))?;
        registry.register(Box::new(api_key_requests.clone()))?;
        let channel_depth = IntGauge::with_opts(Opts::new(
            "gateway_channel_depth",
            "Current depth of internal async channel / queue"
        ))?;
        let channel_capacity = IntGauge::with_opts(Opts::new(
            "gateway_channel_capacity",
            "Configured capacity (static) of internal async channel / queue"
        ))?;
        let channel_high_water = IntGauge::with_opts(Opts::new(
            "gateway_channel_high_water",
            "Observed high-water mark of channel depth since process start"
        ))?;
        registry.register(Box::new(channel_depth.clone()))?;
        registry.register(Box::new(channel_capacity.clone()))?;
        registry.register(Box::new(channel_high_water.clone()))?;
        let rate_limit_latency = Histogram::with_opts(HistogramOpts::new(
            "gateway_rate_limiter_decision_seconds",
            "Time spent performing rate limiter decision (seconds)"
        ))?;
        let rate_window_usage = IntGauge::with_opts(Opts::new(
            "gateway_rate_window_usage",
            "Current count in active rate limit window for last evaluated key"
        ))?;
        let rate_limit_rpm_target = IntGauge::with_opts(Opts::new(
            "gateway_rate_limit_rpm_target",
            "Configured per-identity rate limit requests per minute target"
        ))?;
        let build_info = IntGauge::with_opts(Opts::new(
            "gateway_build_info",
            "Static build info metric (value always 1) with version/hash labels"
        ))?;
        registry.register(Box::new(rate_limit_latency.clone()))?;
        registry.register(Box::new(rate_window_usage.clone()))?;
        registry.register(Box::new(rate_limit_rpm_target.clone()))?;
        registry.register(Box::new(build_info.clone()))?;
        Ok(Self {
            registry,
            rate_checks,
            rate_rejections,
            api_key_requests,
            channel_depth,
            channel_capacity,
            channel_high_water,
            rate_limit_latency,
            rate_window_usage,
            rate_limit_rpm_target,
            build_info,
        })
    }

    pub fn record_rate_check(&self, identity: &str, allowed: bool) {
        self.rate_checks.with_label_values(&[identity]).inc();
        if !allowed {
            self.rate_rejections.with_label_values(&[identity]).inc();
        }
    }

    pub fn record_api_key_request(&self, result: &str) {
        self.api_key_requests.with_label_values(&[result]).inc();
    }

    pub fn render(&self) -> Result<Response> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer)?;
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; version=0.0.4"),
            )
            .body(Body::from(buffer))?;
        Ok(response)
    }

    // Convenience for tests / smoke checks without HTTP layer.
    pub fn gather_text(&self) -> Result<String> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer)?;
        Ok(String::from_utf8(buffer).unwrap_or_default())
    }

    // Backpressure metrics update helpers
    pub fn set_channel_capacity(&self, capacity: usize) {
        self.channel_capacity.set(capacity as i64);
    }

    pub fn update_channel_depth(&self, depth: usize) {
        self.channel_depth.set(depth as i64);
        // Track high-water mark manually (atomic via fetch-update not strictly needed with single-threaded calls)
        if depth as i64 > self.channel_high_water.get() {
            self.channel_high_water.set(depth as i64);
        }
    }

    pub fn observe_rate_limiter_latency(&self, secs: f64) {
        self.rate_limit_latency.observe(secs);
    }

    pub fn set_rate_window_usage(&self, count: i64) {
        self.rate_window_usage.set(count);
    }

    pub fn set_rate_limit_rpm_target(&self, rpm: i64) {
        self.rate_limit_rpm_target.set(rpm);
    }

    pub fn set_build_info(&self) {
        // We encode version/hash as separate metrics due to IntGauge lacking labels in this simple wrapper.
        // Alternative would be to use IntGaugeVec but for simplicity we output two gauge metrics.
        // Set to 1 so presence is assertable.
        let version = env::var("GIT_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
        let hash = env::var("GIT_COMMIT_HASH").unwrap_or_else(|_| "unknown".to_string());
        self.build_info.set(1);
        // We also register ad-hoc gauges for version/hash if desired later; for now rely on environment exposure.
        tracing::info!(version, hash, "gateway build info registered");
    }
}
