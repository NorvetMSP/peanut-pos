use anyhow::{Context, Result};
use std::env;

#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub rate_limit_rpm: u32,
    pub rate_limit_window_secs: u64,
    pub redis_url: String,
    pub redis_prefix: String,
    pub api_usage_flush_secs: u64,
    pub api_usage_summary_secs: u64,
    pub audit_topic: String,
    pub alert_topic: String,
    pub rate_limit_burst_multiplier: f64,
    pub rate_limit_alert_cooldown_secs: u64,
    pub security_alert_webhook_url: Option<String>,
    pub security_alert_webhook_bearer: Option<String>,
}

impl GatewayConfig {
    pub fn from_env() -> Result<Self> {
        let redis_url = env::var("REDIS_URL").context("REDIS_URL must be set")?;
        let rate_limit_rpm = env::var("GATEWAY_RATE_LIMIT_RPM")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(60);
        let rate_limit_window_secs = env::var("GATEWAY_RATE_LIMIT_WINDOW_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(60);
        let redis_prefix = env::var("GATEWAY_RATE_LIMIT_PREFIX")
            .unwrap_or_else(|_| "integration-gateway:rate".to_string());
        let api_usage_flush_secs = env::var("API_KEY_USAGE_FLUSH_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(300);
        let api_usage_summary_secs = env::var("API_KEY_USAGE_SUMMARY_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(3600);
        let audit_topic = env::var("AUDIT_TOPIC").unwrap_or_else(|_| "audit.events.v1".to_string());
        let alert_topic =
            env::var("SECURITY_ALERT_TOPIC").unwrap_or_else(|_| "security.alerts.v1".to_string());
        let rate_limit_burst_multiplier = env::var("GATEWAY_RATE_LIMIT_ALERT_MULTIPLIER")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(3.0);
        let rate_limit_alert_cooldown_secs = env::var("GATEWAY_RATE_LIMIT_ALERT_COOLDOWN_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(300);
        let security_alert_webhook_url = env::var("SECURITY_ALERT_WEBHOOK_URL").ok();
        let security_alert_webhook_bearer = env::var("SECURITY_ALERT_WEBHOOK_BEARER").ok();

        Ok(Self {
            rate_limit_rpm,
            rate_limit_window_secs: rate_limit_window_secs.max(1),
            redis_url,
            redis_prefix,
            api_usage_flush_secs: api_usage_flush_secs.max(60),
            api_usage_summary_secs: api_usage_summary_secs.max(300),
            audit_topic,
            alert_topic,
            rate_limit_burst_multiplier,
            rate_limit_alert_cooldown_secs: rate_limit_alert_cooldown_secs.max(60),
            security_alert_webhook_url,
            security_alert_webhook_bearer,
        })
    }
}
