use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
#[cfg(feature = "kafka")] use rdkafka::producer::{FutureProducer, FutureRecord};
use reqwest::Client;
use serde::Serialize;
use serde_json::json;
#[cfg(feature = "kafka")] use std::time::Duration;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct RateLimitAlertEvent {
    pub action: &'static str,
    pub tenant_id: Option<Uuid>,
    pub key_hash: Option<String>,
    pub key_suffix: Option<String>,
    pub identity: String,
    pub limit: u32,
    pub count: i64,
    pub window_seconds: u64,
    pub occurred_at: DateTime<Utc>,
    pub message: String,
}

#[cfg(feature = "kafka")]
pub async fn publish_rate_limit_alert(producer: &FutureProducer, topic: &str, event: &RateLimitAlertEvent) -> Result<()> {
    if topic.trim().is_empty() { return Ok(()); }
    let payload = serde_json::to_string(event)?;
    let key = event.tenant_id.map(|id| id.to_string()).unwrap_or_else(|| "gateway".to_string());
    producer.send(FutureRecord::to(topic).payload(&payload).key(&key), Duration::from_secs(0)).await.map_err(|(err, _)| anyhow!("Failed to publish rate limit alert: {err}"))?;
    Ok(())
}

#[cfg(not(feature = "kafka"))]
pub async fn publish_rate_limit_alert(_producer: &(), _topic: &str, _event: &RateLimitAlertEvent) -> Result<()> {
    // No-op when kafka disabled
    Ok(())
}

pub async fn post_alert_webhook(
    client: &Client,
    url: &str,
    bearer: Option<&str>,
    text: &str,
) -> Result<()> {
    if url.trim().is_empty() {
        return Ok(());
    }

    let mut req = client.post(url).json(&json!({ "text": text }));
    if let Some(token) = bearer {
        req = req.bearer_auth(token);
    }

    let response = req.send().await?;
    if !response.status().is_success() {
        warn!(status = ?response.status(), "Security webhook returned failure status");
        return Err(anyhow!(
            "Security webhook returned status {}",
            response.status()
        ));
    }

    info!("Posted rate limit alert webhook");
    Ok(())
}
