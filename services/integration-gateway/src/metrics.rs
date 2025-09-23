use anyhow::Result;
use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use prometheus::{Encoder, IntCounterVec, Opts, Registry, TextEncoder};

#[derive(Clone)]
pub struct GatewayMetrics {
    registry: Registry,
    rate_checks: IntCounterVec,
    rate_rejections: IntCounterVec,
    api_key_requests: IntCounterVec,
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
        Ok(Self {
            registry,
            rate_checks,
            rate_rejections,
            api_key_requests,
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
}
