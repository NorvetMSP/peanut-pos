use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use chrono::Utc;
use uuid::Uuid;
use crate::metrics::GatewayMetrics;
use crate::rate_limiter::RateLimiter;
use crate::usage::UsageTracker;
use crate::config::GatewayConfig;
use common_auth::JwtVerifier;
use reqwest::Client;
use tracing::{warn};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::FutureProducer;
use crate::alerts::{post_alert_webhook, RateLimitAlertEvent};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use crate::alerts::publish_rate_limit_alert;

#[derive(Clone)]
pub struct AppState {
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] pub kafka_producer: FutureProducer,
    pub rate_limiter: RateLimiter,
    pub key_cache: Arc<tokio::sync::RwLock<HashMap<String, CachedKey>>>,
    pub jwt_verifier: Arc<JwtVerifier>,
    pub metrics: Arc<GatewayMetrics>,
    pub usage: UsageTracker,
    pub config: Arc<GatewayConfig>,
    pub http_client: Client,
    pub alert_state: Arc<Mutex<HashMap<String, Instant>>>,
}

#[derive(Clone)]
pub struct CachedKey {
    pub tenant_id: Uuid,
    pub key_suffix: String,
}

impl AppState {
    pub fn record_api_key_metric(&self, allowed: bool) {
        let result = if allowed { "allowed" } else { "rejected" };
        self.metrics.record_api_key_request(result);
    }

    pub async fn maybe_alert_rate_limit(
        &self,
        identity: &str,
        tenant_id: Option<Uuid>,
        key_hash: Option<&str>,
        key_suffix: Option<&str>,
        limit: u32,
        current: i64,
    ) {
        let threshold = (limit as f64 * self.config.rate_limit_burst_multiplier).ceil() as i64;
        if current < threshold {
            return;
        }
        let alert_key = key_hash
            .map(|hash| format!("api:{}", hash))
            .unwrap_or_else(|| format!("identity:{}", identity));
        {
            let mut guard = self.alert_state.lock().unwrap();
            let now = Instant::now();
            if let Some(last) = guard.get(&alert_key) {
                if now.duration_since(*last).as_secs() < self.config.rate_limit_alert_cooldown_secs {
                    return;
                }
            }
            guard.insert(alert_key.clone(), now);
        }

        let suffix_display = key_suffix.unwrap_or("-");
        let message = format!(
            "Rate limit burst detected identity={} tenant={:?} key_suffix={} count={} limit={} window={}s",
            identity,
            tenant_id,
            suffix_display,
            current,
            limit,
            self.config.rate_limit_window_secs,
        );

        warn!(
            ?tenant_id,
            key_hash,
            key_suffix,
            identity,
            current,
            limit,
            window = self.config.rate_limit_window_secs,
            message,
            "Rate limit burst detected"
        );

        #[allow(unused_variables)]
        let event = RateLimitAlertEvent {
            action: "gateway.rate_limit.alert",
            tenant_id,
            key_hash: key_hash.map(|value| value.to_string()),
            key_suffix: key_suffix.map(|value| value.to_string()),
            identity: identity.to_string(),
            limit,
            count: current,
            window_seconds: self.config.rate_limit_window_secs,
            occurred_at: Utc::now(),
            message: message.clone(),
        };
        #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
        if let Err(err) = publish_rate_limit_alert(&self.kafka_producer, &self.config.alert_topic, &event).await {
            warn!(?err, "Failed to publish rate limit alert");
        }

        if let Some(url) = &self.config.security_alert_webhook_url {
            if let Err(err) = post_alert_webhook(
                &self.http_client,
                url,
                self.config.security_alert_webhook_bearer.as_deref(),
                &message,
            )
            .await
            {
                warn!(?err, "Failed to post security alert webhook");
            }
        }
    }
}
