use anyhow::Result;
use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use prometheus::{Encoder, IntCounterVec, Opts, Registry, TextEncoder};

#[derive(Clone)]
pub struct AuthMetrics {
    registry: Registry,
    login_attempts: IntCounterVec,
    mfa_events: IntCounterVec,
}

impl AuthMetrics {
    pub fn new() -> Result<Self> {
        let registry = Registry::new();

        let login_attempts = IntCounterVec::new(
            Opts::new(
                "auth_login_attempts_total",
                "Count of login attempts grouped by outcome",
            ),
            &["outcome"],
        )?;
        registry.register(Box::new(login_attempts.clone()))?;

        let mfa_events = IntCounterVec::new(
            Opts::new("auth_mfa_events_total", "Count of MFA-related events"),
            &["event"],
        )?;
        registry.register(Box::new(mfa_events.clone()))?;

        Ok(Self {
            registry,
            login_attempts,
            mfa_events,
        })
    }

    pub fn login_attempt(&self, outcome: &str) {
        self.login_attempts.with_label_values(&[outcome]).inc();
    }

    pub fn mfa_event(&self, event: &str) {
        self.mfa_events.with_label_values(&[event]).inc();
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
