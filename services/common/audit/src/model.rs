use chrono::{DateTime,Utc};
use serde::{Serialize,Deserialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuditActor {
    pub id: Option<Uuid>,
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub actor: AuditActor,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub action: String,
    pub occurred_at: DateTime<Utc>,
    pub changes: serde_json::Value,
    pub meta: serde_json::Value,
}

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("producer not configured")] 
    NotConfigured,
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("kafka error: {0}")]
    Kafka(String),
}

pub type AuditResult<T> = Result<T, AuditError>;
