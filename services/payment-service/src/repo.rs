use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use chrono::{DateTime, Utc};
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntentState {
    Created,
    Authorized,
    Captured,
    Refunded,
    Voided,
    Failed,
}

impl IntentState {
    pub fn as_str(&self) -> &'static str {
        match self {
            IntentState::Created => "created",
            IntentState::Authorized => "authorized",
            IntentState::Captured => "captured",
            IntentState::Refunded => "refunded",
            IntentState::Voided => "voided",
            IntentState::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Option<IntentState> {
        match s {
            "created" => Some(IntentState::Created),
            "authorized" => Some(IntentState::Authorized),
            "captured" => Some(IntentState::Captured),
            "refunded" => Some(IntentState::Refunded),
            "voided" => Some(IntentState::Voided),
            "failed" => Some(IntentState::Failed),
            _ => None,
        }
    }
}

/// Valid transitions for MVP:
/// created -> authorized
/// authorized -> captured | voided
/// captured -> refunded
/// Any other transition is invalid and should return HTTP 409
pub fn is_valid_transition(from_state: &str, to: IntentState) -> bool {
    match IntentState::from_str(from_state) {
        Some(IntentState::Created) => matches!(to, IntentState::Authorized),
        Some(IntentState::Authorized) => matches!(to, IntentState::Captured | IntentState::Voided),
        Some(IntentState::Captured) => matches!(to, IntentState::Refunded),
        // Terminal states: refunded/voided/failed cannot transition further in MVP
        Some(IntentState::Refunded | IntentState::Voided | IntentState::Failed) => false,
        None => false,
    }
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct PaymentIntent {
    pub id: String,
    pub order_id: String,
    pub amount_minor: i64,
    pub currency: String,
    pub state: String,
    pub provider: Option<String>,
    pub provider_ref: Option<String>,
    pub idempotency_key: Option<String>,
    pub metadata_json: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn get_intent(db: &PgPool, id: &str) -> Result<Option<PaymentIntent>> {
    let rec = sqlx::query_as::<_, PaymentIntent>(
        r#"SELECT id, order_id, amount_minor, currency, state, provider, provider_ref, idempotency_key, metadata_json, created_at, updated_at
           FROM payment_intents WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(rec)
}

pub async fn create_intent(
    db: &PgPool,
    id: &str,
    order_id: &str,
    amount_minor: i64,
    currency: &str,
    idempotency_key: Option<&str>,
) -> Result<PaymentIntent> {
    let rec = sqlx::query_as::<_, PaymentIntent>(
        r#"INSERT INTO payment_intents (id, order_id, amount_minor, currency, state, idempotency_key)
           VALUES ($1, $2, $3, $4, 'created', $5)
           ON CONFLICT (id) DO UPDATE SET updated_at = now()
           RETURNING id, order_id, amount_minor, currency, state, provider, provider_ref, idempotency_key, metadata_json, created_at, updated_at"#,
    )
    .bind(id)
    .bind(order_id)
    .bind(amount_minor)
    .bind(currency)
    .bind(idempotency_key)
    .fetch_one(db)
    .await?;
    Ok(rec)
}

pub async fn transition_state(db: &PgPool, id: &str, new_state: IntentState) -> Result<Option<PaymentIntent>> {
    let rec = sqlx::query_as::<_, PaymentIntent>(
        r#"UPDATE payment_intents SET state = $2, updated_at = now()
           WHERE id = $1
           RETURNING id, order_id, amount_minor, currency, state, provider, provider_ref, idempotency_key, metadata_json, created_at, updated_at"#,
    )
    .bind(id)
    .bind(new_state.as_str())
    .fetch_optional(db)
    .await?;
    Ok(rec)
}

pub async fn transition_with_provider(
    db: &PgPool,
    id: &str,
    new_state: IntentState,
    provider: Option<&str>,
    provider_ref: Option<&str>,
    metadata_json: Option<&serde_json::Value>,
) -> Result<Option<PaymentIntent>> {
    let rec = sqlx::query_as::<_, PaymentIntent>(
        r#"UPDATE payment_intents
           SET state = $2,
               provider = COALESCE($3, provider),
               provider_ref = COALESCE($4, provider_ref),
               metadata_json = COALESCE($5, metadata_json),
               updated_at = now()
           WHERE id = $1
           RETURNING id, order_id, amount_minor, currency, state, provider, provider_ref, idempotency_key, metadata_json, created_at, updated_at"#,
    )
    .bind(id)
    .bind(new_state.as_str())
    .bind(provider)
    .bind(provider_ref)
    .bind(metadata_json)
    .fetch_optional(db)
    .await?;
    Ok(rec)
}
