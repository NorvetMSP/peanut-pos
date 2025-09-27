use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rdkafka::producer::{FutureProducer, FutureRecord};
use reqwest::Client;
use serde::Serialize;
use std::time::Duration;
use uuid::Uuid;

#[async_trait]
pub trait KafkaProducer: Send + Sync {
    async fn send(&self, topic: &str, key: &str, payload: String) -> Result<()>;
}

#[async_trait]
impl KafkaProducer for FutureProducer {
    async fn send(&self, topic: &str, key: &str, payload: String) -> Result<()> {
        self.send(
            FutureRecord::to(topic).payload(&payload).key(key),
            Duration::from_secs(0),
        )
        .await
        .map_err(|(err, _)| anyhow!("Failed to publish MFA activity: {err}"))?;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct MfaActivityEvent {
    pub action: &'static str,
    pub severity: &'static str,
    pub tenant_id: Uuid,
    pub user_id: Option<Uuid>,
    pub trace_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SuspiciousLoginPayload {
    pub text: String,
}

pub async fn publish_mfa_activity(
    producer: &dyn KafkaProducer,
    topic: &str,
    event: &MfaActivityEvent,
) -> Result<()> {
    if topic.trim().is_empty() {
        return Ok(());
    }

    let payload = serde_json::to_string(event)?;
    let key = event.tenant_id.to_string();
    producer.send(topic, &key, payload).await
}

pub async fn post_suspicious_webhook(
    client: &Client,
    url: &str,
    bearer: Option<&str>,
    payload: &SuspiciousLoginPayload,
) -> Result<()> {
    let mut request = client.post(url).json(payload);
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "Security webhook returned status {}",
            response.status()
        ));
    }
    Ok(())
}
