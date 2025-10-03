use crate::config::GatewayConfig;
use anyhow::Result;
use chrono::{DateTime, Utc};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::{FutureProducer, FutureRecord};
use serde::Serialize;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use std::time::Duration as StdDuration;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration, MissedTickBehavior};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use tracing::error;
use tracing::warn;
use uuid::Uuid;

#[derive(Clone)]
pub struct UsageTracker {
    inner: Arc<UsageTrackerInner>,
}

struct UsageTrackerInner {
    pool: PgPool,
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] producer: FutureProducer,
    topic: String,
    flush_secs: u64,
    summary_secs: u64,
    data: Mutex<HashMap<String, UsageRecord>>,
}

struct UsageRecord {
    tenant_id: Uuid,
    key_hash: String,
    key_suffix: String,
    window_start: DateTime<Utc>,
    window_count: u64,
    window_rejected: u64,
    last_seen: DateTime<Utc>,
    summary_start: DateTime<Utc>,
    summary_count: u64,
    summary_rejected: u64,
}

#[allow(dead_code)]
struct UsageWindow {
    tenant_id: Uuid,
    key_hash: String,
    key_suffix: String,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    request_count: u64,
    rejected_count: u64,
    last_seen: DateTime<Utc>,
}

#[allow(dead_code)]
struct UsageSummary {
    tenant_id: Uuid,
    key_hash: String,
    key_suffix: String,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    request_count: u64,
    rejected_count: u64,
}

#[derive(Serialize)]
struct ApiKeyUsageSummary {
    action: &'static str,
    tenant_id: Uuid,
    key_hash: String,
    key_suffix: String,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    request_count: u64,
    rejected_count: u64,
}

impl UsageTracker {
    pub fn new(config: Arc<GatewayConfig>, pool: PgPool,
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] producer: Option<FutureProducer>
    ) -> Self {
        Self { inner: Arc::new(UsageTrackerInner { pool,
            #[cfg(any(feature = "kafka", feature = "kafka-producer"))] producer: producer.expect("producer required when kafka feature enabled"),
            topic: config.audit_topic.clone(),
            flush_secs: config.api_usage_flush_secs,
            summary_secs: config.api_usage_summary_secs,
            data: Mutex::new(HashMap::new()), }) }
    }

    pub fn spawn_background_tasks(&self) {
        let flush_self = self.clone();
        let flush_interval = Duration::from_secs(self.inner.flush_secs);
        tokio::spawn(async move {
            let mut ticker = interval(flush_interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if let Err(err) = flush_self.flush_window().await {
                    warn!(?err, "Failed to flush API key usage window");
                }
            }
        });

        let summary_self = self.clone();
        let summary_interval = Duration::from_secs(self.inner.summary_secs);
        tokio::spawn(async move {
            let mut ticker = interval(summary_interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if let Err(err) = summary_self.flush_summary().await {
                    warn!(?err, "Failed to emit API key usage summary");
                }
            }
        });
    }

    pub async fn record_api_key_use(
        &self,
        tenant_id: Uuid,
        key_hash: &str,
        key_suffix: &str,
        allowed: bool,
    ) {
        let now = Utc::now();
        let mut guard = self.inner.data.lock().await;
        let entry = guard
            .entry(key_hash.to_string())
            .or_insert_with(|| UsageRecord {
                tenant_id,
                key_hash: key_hash.to_string(),
                key_suffix: key_suffix.to_string(),
                window_start: now,
                window_count: 0,
                window_rejected: 0,
                last_seen: now,
                summary_start: now,
                summary_count: 0,
                summary_rejected: 0,
            });

        entry.window_count += 1;
        entry.summary_count += 1;
        if !allowed {
            entry.window_rejected += 1;
            entry.summary_rejected += 1;
        }
        entry.last_seen = now;
    }

    async fn flush_window(&self) -> Result<()> {
        let now = Utc::now();
        let mut windows = Vec::new();
        {
            let mut guard = self.inner.data.lock().await;
            for record in guard.values_mut() {
                if record.window_count == 0 {
                    continue;
                }
                windows.push(UsageWindow {
                    tenant_id: record.tenant_id,
                    key_hash: record.key_hash.clone(),
                    key_suffix: record.key_suffix.clone(),
                    window_start: record.window_start,
                    window_end: now,
                    request_count: record.window_count,
                    rejected_count: record.window_rejected,
                    last_seen: record.last_seen,
                });
                record.window_start = now;
                record.window_count = 0;
                record.window_rejected = 0;
            }
        }

        if windows.is_empty() {
            return Ok(());
        }

        for window in windows {
            sqlx::query(
                "INSERT INTO api_key_usage (tenant_id, key_hash, key_suffix, window_start, window_end, request_count, rejected_count, last_seen_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (tenant_id, key_hash, window_start)
                 DO UPDATE SET request_count = api_key_usage.request_count + EXCLUDED.request_count,
                               rejected_count = api_key_usage.rejected_count + EXCLUDED.rejected_count,
                               window_end = EXCLUDED.window_end,
                               last_seen_at = GREATEST(api_key_usage.last_seen_at, EXCLUDED.last_seen_at)",
            )
            .bind(window.tenant_id)
            .bind(&window.key_hash)
            .bind(&window.key_suffix)
            .bind(window.window_start)
            .bind(window.window_end)
            .bind(window.request_count as i64)
            .bind(window.rejected_count as i64)
            .bind(window.last_seen)
            .execute(&self.inner.pool)
            .await?;
        }
        Ok(())
    }

    async fn flush_summary(&self) -> Result<()> {
        let now = Utc::now();
        let mut summaries = Vec::new();
        {
            let mut guard = self.inner.data.lock().await;
            for record in guard.values_mut() {
                if record.summary_count == 0 {
                    continue;
                }
                summaries.push(UsageSummary {
                    tenant_id: record.tenant_id,
                    key_hash: record.key_hash.clone(),
                    key_suffix: record.key_suffix.clone(),
                    window_start: record.summary_start,
                    window_end: now,
                    request_count: record.summary_count,
                    rejected_count: record.summary_rejected,
                });
                record.summary_start = now;
                record.summary_count = 0;
                record.summary_rejected = 0;
            }
        }

        if summaries.is_empty() {
            return Ok(());
        }

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
        {
            for summary in summaries {
                let event = ApiKeyUsageSummary { action: "api_key.usage.summary", tenant_id: summary.tenant_id, key_hash: summary.key_hash.clone(), key_suffix: summary.key_suffix.clone(), window_start: summary.window_start, window_end: summary.window_end, request_count: summary.request_count, rejected_count: summary.rejected_count };
                match serde_json::to_string(&event) {
                    Ok(payload) => {
                        if let Err(err) = self.inner.producer.send(FutureRecord::to(&self.inner.topic).payload(&payload).key(&summary.tenant_id.to_string()), StdDuration::from_secs(0)).await {
                            error!(?err, tenant_id = %summary.tenant_id, key = %summary.key_hash, "Failed to publish API key usage summary");
                        }
                    }
                    Err(err) => error!(?err, "Failed to serialize API key usage summary"),
                }
            }
        }
        #[cfg(not(feature = "kafka"))]
        {
            // No-op when kafka disabled.
        }
        Ok(())
    }
}
