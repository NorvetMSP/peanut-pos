use crate::{AuditActor, AuditEvent, AuditError, AuditResult, AUDIT_EVENT_VERSION, AuditSeverity};
use chrono::Utc;
use rdkafka::producer::{FutureProducer, FutureRecord};
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone)]
pub struct AuditProducerConfig {
    pub topic: String,
}

#[derive(Clone)]
pub struct AuditProducer {
    inner: Option<FutureProducer>,
    config: AuditProducerConfig,
}

impl AuditProducer {
    pub fn new(inner: Option<FutureProducer>, config: AuditProducerConfig) -> Self { Self { inner, config } }

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
        let Some(producer) = &self.inner else { return Err(AuditError::NotConfigured); };
        let serialized = serde_json::to_vec(&event).map_err(|e| AuditError::Serialization(e.to_string()))?;
        let key = event.tenant_id.to_string();
        let record = FutureRecord::to(&self.config.topic)
            .key(&key)
            .payload(&serialized);
        if let Err((e,_)) = producer.send(record, Duration::from_secs(5)).await { return Err(AuditError::Kafka(e.to_string())); }
        Ok(event)
    }

    pub fn dummy(topic: &str) -> Self { Self { inner: None, config: AuditProducerConfig { topic: topic.to_string() } } }
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
