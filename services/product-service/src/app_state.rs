use std::sync::Arc;
use rdkafka::producer::FutureProducer;
use sqlx::PgPool;
use common_auth::JwtVerifier;
use axum::extract::FromRef;

/// Shared application state used by handlers (moved from main.rs so tests & library code can reference it).
#[derive(Clone)]
pub struct AppState {
    pub(crate) db: PgPool,
    pub(crate) kafka_producer: FutureProducer,
    pub(crate) jwt_verifier: Arc<JwtVerifier>,
    pub(crate) audit_producer: Option<Arc<common_audit::BufferedAuditProducer<common_audit::KafkaAuditSink>>>,
}

impl AppState {
    pub fn new(db: PgPool, kafka_producer: FutureProducer, jwt_verifier: Arc<JwtVerifier>, audit_producer: Option<Arc<common_audit::BufferedAuditProducer<common_audit::KafkaAuditSink>>>) -> Self {
        Self { db, kafka_producer, jwt_verifier, audit_producer }
    }
    pub fn audit_buffer(&self) -> Option<&Arc<common_audit::BufferedAuditProducer<common_audit::KafkaAuditSink>>> { self.audit_producer.as_ref() }
}

impl FromRef<AppState> for Arc<JwtVerifier> {
    fn from_ref(state: &AppState) -> Self { state.jwt_verifier.clone() }
}
