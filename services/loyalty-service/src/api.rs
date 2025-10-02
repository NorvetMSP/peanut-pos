use axum::extract::{State, Query};
use std::collections::HashMap;
use common_http_errors::ApiError;
use common_security::{SecurityCtxExtractor, Capability, ensure_capability};
use uuid::Uuid;
use sqlx::PgPool;
use std::sync::Arc;
use common_auth::JwtVerifier;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt_verifier: Arc<JwtVerifier>,
    #[cfg(feature = "kafka")] pub producer: rdkafka::producer::FutureProducer,
}

// Legacy LOYALTY_VIEW_ROLES removed; authorization via LoyaltyView capability only.

pub async fn get_points(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Query(params): Query<HashMap<String, String>>,
) -> Result<String, ApiError> {
    ensure_capability(&sec, Capability::LoyaltyView)
        .map_err(|_| ApiError::ForbiddenMissingRole { role: "loyalty_view", trace_id: sec.trace_id })?;
    let tenant_id = sec.tenant_id;
    let cust_id = params
        .get("customer_id")
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or(ApiError::BadRequest { code: "missing_customer_id", trace_id: sec.trace_id, message: Some("customer_id required".into()) })?;

    let rec = sqlx::query!(
        r#"SELECT points FROM loyalty_points WHERE customer_id = $1 AND tenant_id = $2"#,
        cust_id,
        tenant_id
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: sec.trace_id, message: Some(format!("DB error: {e}")) })?;

    Ok(rec.points.to_string())
}
