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

pub const AUDIT_EVENT_VERSION: i32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditSeverity { Info, Warning, Security, Compliance }

impl Default for AuditSeverity { fn default() -> Self { AuditSeverity::Info } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: Uuid,
    pub event_version: i32,
    pub tenant_id: Uuid,
    pub actor: AuditActor,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub action: String,
    pub occurred_at: DateTime<Utc>,
    pub source_service: String,
    pub severity: AuditSeverity,
    pub trace_id: Option<Uuid>,
    pub payload: serde_json::Value,
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
