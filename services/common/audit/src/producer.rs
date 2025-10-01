use crate::{AuditActor, AuditEvent, AuditError, AuditResult, AUDIT_EVENT_VERSION, AuditSeverity};
use chrono::Utc;
use uuid::Uuid;
#[cfg(feature = "kafka")] use rdkafka::producer::{FutureProducer, FutureRecord};
#[cfg(feature = "kafka")] use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AuditProducerConfig { pub topic: String }

#[async_trait::async_trait]
pub trait AuditSink: Send + Sync + 'static {
    async fn emit(&self, event: AuditEvent) -> AuditResult<()>;
}

#[derive(Clone)]
pub struct KafkaAuditSink { #[cfg(feature = "kafka")] pub(crate) inner: FutureProducer, pub(crate) config: AuditProducerConfig }

impl KafkaAuditSink {
    #[cfg(feature = "kafka")]
    pub fn new(inner: FutureProducer, config: AuditProducerConfig) -> Self { Self { inner, config } }
}

#[cfg(feature = "kafka")]
#[async_trait::async_trait]
impl AuditSink for KafkaAuditSink {
    async fn emit(&self, event: AuditEvent) -> AuditResult<()> {
        let serialized = serde_json::to_vec(&event).map_err(|e| AuditError::Serialization(e.to_string()))?;
        let key = event.tenant_id.to_string();
        let record = FutureRecord::to(&self.config.topic).key(&key).payload(&serialized);
        if let Err((e,_)) = self.inner.send(record, Duration::from_secs(5)).await { return Err(AuditError::Kafka(e.to_string())); }
        Ok(())
    }
}

pub struct NoopAuditSink;

#[async_trait::async_trait]
impl AuditSink for NoopAuditSink { async fn emit(&self, _event: AuditEvent) -> AuditResult<()> { Ok(()) } }

#[derive(Clone)]
pub struct AuditProducer<S: AuditSink> { sink: S }

impl<S: AuditSink> AuditProducer<S> {
    pub fn new(sink: S) -> Self { Self { sink } }

    pub async fn emit(
        &self,
        tenant_id: Uuid,
        actor: AuditActor,
        entity_type: impl Into<String>,
        entity_id: Option<Uuid>,
        action: impl Into<String>,
        source_service: &str,
        severity: AuditSeverity,
        trace_id: Option<Uuid>,
        payload: serde_json::Value,
        meta: serde_json::Value,
    ) -> AuditResult<AuditEvent> {
        let event = AuditEvent {
            event_id: Uuid::new_v4(),
            event_version: AUDIT_EVENT_VERSION,
            tenant_id,
            actor: actor.clone(),
            entity_type: entity_type.into(),
            entity_id,
            action: action.into(),
            occurred_at: Utc::now(),
            source_service: source_service.to_string(),
            severity,
            trace_id,
            payload,
            meta,
        };
        self.sink.emit(event.clone()).await?;
        Ok(event)
    }
}

pub fn extract_actor_from_headers(headers: &axum::http::HeaderMap, claims_raw: &serde_json::Value, subject: uuid::Uuid) -> AuditActor {
    use axum::http::HeaderMap;
    fn header_str(map: &HeaderMap, name: &str) -> Option<String> { map.get(name).and_then(|v| v.to_str().ok()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) }
    let mut actor = AuditActor { id: Some(subject), name: None, email: None };
    actor.name = claims_raw.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
    actor.email = claims_raw.get("email").and_then(|v| v.as_str()).map(|s| s.to_string());
    if let Some(v) = header_str(headers, "X-User-ID").and_then(|s| uuid::Uuid::parse_str(&s).ok()) { actor.id = Some(v); }
    if let Some(v) = header_str(headers, "X-User-Name") { actor.name = Some(v); }
    if let Some(v) = header_str(headers, "X-User-Email") { actor.email = Some(v); }
    actor
}
