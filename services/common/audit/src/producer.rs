use crate::{AuditActor, AuditEvent, AuditResult, AUDIT_EVENT_VERSION, AuditSeverity};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use crate::AuditError;
use chrono::Utc;
use uuid::Uuid;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::{FutureProducer, FutureRecord};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

#[derive(Debug, Clone)]
pub struct AuditProducerConfig { pub topic: String }

#[async_trait::async_trait]
pub trait AuditSink: Send + Sync + 'static {
    async fn emit(&self, event: AuditEvent) -> AuditResult<()>;
}

// Real Kafka sink implementation (requires rdkafka)
#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
#[derive(Clone)]
pub struct KafkaAuditSink { pub(crate) inner: FutureProducer, pub(crate) config: AuditProducerConfig }

#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
impl KafkaAuditSink {
    pub fn new(inner: FutureProducer, config: AuditProducerConfig) -> Self { Self { inner, config } }
}

#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
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

// Stub KafkaAuditSink for kafka-core feature (compile-time type presence, no rdkafka linkage)
#[cfg(all(feature = "kafka-core", not(any(feature = "kafka", feature = "kafka-producer"))))]
#[derive(Clone)]
pub struct KafkaAuditSink { pub(crate) config: AuditProducerConfig }

#[cfg(all(feature = "kafka-core", not(any(feature = "kafka", feature = "kafka-producer"))))]
impl KafkaAuditSink {
    pub fn new<T>(_inner: T, config: AuditProducerConfig) -> Self { Self { config } }
}

#[cfg(all(feature = "kafka-core", not(any(feature = "kafka", feature = "kafka-producer"))))]
#[async_trait::async_trait]
impl AuditSink for KafkaAuditSink { async fn emit(&self, _event: AuditEvent) -> AuditResult<()> { Ok(()) } }

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

/// Buffered wrapper around an inner AuditProducer to reduce per-request latency.
pub struct BufferedAuditProducer<S: AuditSink> {
    _inner: Arc<AuditProducer<S>>,
    tx: mpsc::Sender<AuditEvent>,
    _bg: JoinHandle<()>,
    pub queued: Arc<AtomicU64>,
    pub dropped: Arc<AtomicU64>,
    pub emitted: Arc<AtomicU64>,
}

impl<S: AuditSink> BufferedAuditProducer<S> {
    pub fn new(inner: AuditProducer<S>, capacity: usize) -> Self {
        let inner = Arc::new(inner);
        let (tx, mut rx) = mpsc::channel::<AuditEvent>(capacity);
        let queued = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let emitted = Arc::new(AtomicU64::new(0));
        let q_clone = queued.clone();
        let e_clone = emitted.clone();
        let clone = inner.clone();
        let bg = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(e) = clone.sink.emit(event).await { tracing::warn!(?e, "audit buffer emit failed"); }
                e_clone.fetch_add(1, Ordering::Relaxed);
                q_clone.fetch_sub(1, Ordering::Relaxed);
            }
        });
    Self { _inner: inner, tx, _bg: bg, queued, dropped, emitted }
    }

    /// Return a point-in-time snapshot of internal counters for external metrics scraping.
    pub fn snapshot(&self) -> BufferedAuditSnapshot {
        BufferedAuditSnapshot {
            queued: self.queued.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            emitted: self.emitted.load(Ordering::Relaxed),
        }
    }

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
    ) -> AuditResult<()> {
        let event = AuditEvent {
            event_id: Uuid::new_v4(),
            event_version: AUDIT_EVENT_VERSION,
            tenant_id,
            actor,
            entity_type: entity_type.into(),
            entity_id,
            action: action.into(),
            occurred_at: chrono::Utc::now(),
            source_service: source_service.to_string(),
            severity,
            trace_id,
            payload,
            meta,
        };
        if let Err(_e) = self.tx.try_send(event) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("audit buffer full; dropping event");
        }
        else {
            self.queued.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
}

/// Plain snapshot structure for Prometheus exposition without holding references.
pub struct BufferedAuditSnapshot {
    pub queued: u64,
    pub dropped: u64,
    pub emitted: u64,
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
