use crate::{AppState, repo};
use axum::{
    extract::State,
    http::HeaderMap,
    Json,
};
use common_security::{SecurityCtxExtractor, Capability, ensure_capability};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use common_security::emit_capability_denial_audit;
use common_http_errors::ApiError;
use serde::{Deserialize, Serialize};
use bigdecimal::BigDecimal;
use common_money::Money; // normalize_scale not needed here
use std::time::Duration;
use tokio::time::sleep;

#[derive(Deserialize)]
pub struct PaymentRequest {
    #[serde(rename = "orderId")]
    pub order_id: String,
    #[allow(dead_code)]
    pub method: String,
    pub amount: BigDecimal,
}
// Legacy PAYMENT_ROLES removed: rely solely on PaymentProcess capability (Cashier + Admin-like roles allowed by mapping).
#[derive(Serialize)]
pub struct PaymentResponse {
    pub status: String,
    pub approval_code: String,
}

#[derive(Deserialize)]
pub struct VoidPaymentRequest {
    #[serde(rename = "orderId")]
    pub order_id: String,
    #[allow(dead_code)]
    pub method: String,
    pub amount: BigDecimal,
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct VoidPaymentResponse {
    pub status: String,
    pub approval_code: String,
}

// --- Payment Intents MVP (stubs) ---
#[derive(Deserialize, Serialize)]
pub struct CreateIntentRequest {
    pub id: String,
    #[serde(rename = "orderId")] pub order_id: String,
    #[serde(rename = "amountMinor")] pub amount_minor: i64,
    pub currency: String,
    #[serde(rename = "idempotencyKey")] pub idempotency_key: Option<String>,
}

#[derive(Serialize)]
pub struct IntentResponse {
    pub id: String,
    pub state: String,
}

#[allow(unused_variables)]
pub async fn create_intent(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    _headers: HeaderMap,
    Json(req): Json<CreateIntentRequest>,
) -> Result<Json<IntentResponse>, ApiError> {
    if ensure_capability(&sec, Capability::PaymentProcess).is_err() {
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] emit_capability_denial_audit(state.audit_producer.as_deref(), &sec, Capability::PaymentProcess, "payment-service").await;
        return Err(ApiError::ForbiddenMissingRole { role: "payment_access", trace_id: sec.trace_id });
    }
    if let Some(db) = &state.db {
        let rec = repo::create_intent(db, &req.id, &req.order_id, req.amount_minor, &req.currency, req.idempotency_key.as_deref()).await
            .map_err(|e| ApiError::Internal { trace_id: sec.trace_id, message: Some(format!("db_error: {e}")) })?;
        return Ok(Json(IntentResponse { id: rec.id, state: rec.state }));
    }
    // Fallback: no DB configured
    Ok(Json(IntentResponse { id: req.id, state: "created".into() }))
}

#[derive(Deserialize)]
pub struct ConfirmIntentRequest {
    pub id: String,
    pub provider: Option<String>,
    #[serde(rename = "providerRef")] pub provider_ref: Option<String>,
    #[serde(rename = "metadata")] pub metadata_json: Option<serde_json::Value>,
}

#[allow(unused_variables)]
pub async fn confirm_intent(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    _headers: HeaderMap,
    Json(req): Json<ConfirmIntentRequest>,
) -> Result<Json<IntentResponse>, ApiError> {
    if ensure_capability(&sec, Capability::PaymentProcess).is_err() {
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] emit_capability_denial_audit(state.audit_producer.as_deref(), &sec, Capability::PaymentProcess, "payment-service").await;
        return Err(ApiError::ForbiddenMissingRole { role: "payment_access", trace_id: sec.trace_id });
    }
    if let Some(db) = &state.db {
        // Fetch current state and validate transition
        let existing = repo::get_intent(db, &req.id).await
            .map_err(|e| ApiError::Internal { trace_id: sec.trace_id, message: Some(format!("db_error: {e}")) })?;
        let Some(cur) = existing else { return Err(ApiError::NotFound { code: "payment_intent_not_found", trace_id: sec.trace_id }); };
        if !repo::is_valid_transition(&cur.state, repo::IntentState::Authorized) {
            return Err(ApiError::Conflict { code: "invalid_state_transition", trace_id: sec.trace_id, message: Some(format!("from={} to=authorized", cur.state)) });
        }
        let rec = repo::transition_with_provider(db, &req.id, repo::IntentState::Authorized, req.provider.as_deref(), req.provider_ref.as_deref(), req.metadata_json.as_ref()).await
            .map_err(|e| ApiError::Internal { trace_id: sec.trace_id, message: Some(format!("db_error: {e}")) })?;
        if let Some(pi) = rec { return Ok(Json(IntentResponse { id: pi.id, state: pi.state })); }
        return Err(ApiError::NotFound { code: "payment_intent_not_found", trace_id: sec.trace_id });
    }
    Ok(Json(IntentResponse { id: req.id, state: "authorized".into() }))
}

#[derive(Deserialize)]
pub struct CaptureIntentRequest {
    pub id: String,
    pub provider: Option<String>,
    #[serde(rename = "providerRef")] pub provider_ref: Option<String>,
    #[serde(rename = "metadata")] pub metadata_json: Option<serde_json::Value>,
}

#[allow(unused_variables)]
pub async fn capture_intent(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    _headers: HeaderMap,
    Json(req): Json<CaptureIntentRequest>,
) -> Result<Json<IntentResponse>, ApiError> {
    if ensure_capability(&sec, Capability::PaymentProcess).is_err() {
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] emit_capability_denial_audit(state.audit_producer.as_deref(), &sec, Capability::PaymentProcess, "payment-service").await;
        return Err(ApiError::ForbiddenMissingRole { role: "payment_access", trace_id: sec.trace_id });
    }
    if let Some(db) = &state.db {
        let existing = repo::get_intent(db, &req.id).await
            .map_err(|e| ApiError::Internal { trace_id: sec.trace_id, message: Some(format!("db_error: {e}")) })?;
        let Some(cur) = existing else { return Err(ApiError::NotFound { code: "payment_intent_not_found", trace_id: sec.trace_id }); };
        if !repo::is_valid_transition(&cur.state, repo::IntentState::Captured) {
            return Err(ApiError::Conflict { code: "invalid_state_transition", trace_id: sec.trace_id, message: Some(format!("from={} to=captured", cur.state)) });
        }
        let rec = repo::transition_with_provider(db, &req.id, repo::IntentState::Captured, req.provider.as_deref(), req.provider_ref.as_deref(), req.metadata_json.as_ref()).await
            .map_err(|e| ApiError::Internal { trace_id: sec.trace_id, message: Some(format!("db_error: {e}")) })?;
        if let Some(pi) = rec { return Ok(Json(IntentResponse { id: pi.id, state: pi.state })); }
        return Err(ApiError::NotFound { code: "payment_intent_not_found", trace_id: sec.trace_id });
    }
    Ok(Json(IntentResponse { id: req.id, state: "captured".into() }))
}

#[derive(Deserialize)]
pub struct VoidIntentRequest { pub id: String }

#[allow(unused_variables)]
pub async fn void_intent(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    _headers: HeaderMap,
    Json(req): Json<VoidIntentRequest>,
) -> Result<Json<IntentResponse>, ApiError> {
    if ensure_capability(&sec, Capability::PaymentProcess).is_err() {
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] emit_capability_denial_audit(state.audit_producer.as_deref(), &sec, Capability::PaymentProcess, "payment-service").await;
        return Err(ApiError::ForbiddenMissingRole { role: "payment_access", trace_id: sec.trace_id });
    }
    if let Some(db) = &state.db {
        let existing = repo::get_intent(db, &req.id).await
            .map_err(|e| ApiError::Internal { trace_id: sec.trace_id, message: Some(format!("db_error: {e}")) })?;
        let Some(cur) = existing else { return Err(ApiError::NotFound { code: "payment_intent_not_found", trace_id: sec.trace_id }); };
        if !repo::is_valid_transition(&cur.state, repo::IntentState::Voided) {
            return Err(ApiError::Conflict { code: "invalid_state_transition", trace_id: sec.trace_id, message: Some(format!("from={} to=voided", cur.state)) });
        }
        let rec = repo::transition_state(db, &req.id, repo::IntentState::Voided).await
            .map_err(|e| ApiError::Internal { trace_id: sec.trace_id, message: Some(format!("db_error: {e}")) })?;
        if let Some(pi) = rec { return Ok(Json(IntentResponse { id: pi.id, state: pi.state })); }
        return Err(ApiError::NotFound { code: "payment_intent_not_found", trace_id: sec.trace_id });
    }
    Ok(Json(IntentResponse { id: req.id, state: "voided".into() }))
}

#[derive(Deserialize)]
pub struct RefundIntentRequest { pub id: String }

#[allow(unused_variables)]
pub async fn refund_intent(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    _headers: HeaderMap,
    Json(req): Json<RefundIntentRequest>,
) -> Result<Json<IntentResponse>, ApiError> {
    if ensure_capability(&sec, Capability::PaymentProcess).is_err() {
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] emit_capability_denial_audit(state.audit_producer.as_deref(), &sec, Capability::PaymentProcess, "payment-service").await;
        return Err(ApiError::ForbiddenMissingRole { role: "payment_access", trace_id: sec.trace_id });
    }
    if let Some(db) = &state.db {
        let existing = repo::get_intent(db, &req.id).await
            .map_err(|e| ApiError::Internal { trace_id: sec.trace_id, message: Some(format!("db_error: {e}")) })?;
        let Some(cur) = existing else { return Err(ApiError::NotFound { code: "payment_intent_not_found", trace_id: sec.trace_id }); };
        if !repo::is_valid_transition(&cur.state, repo::IntentState::Refunded) {
            return Err(ApiError::Conflict { code: "invalid_state_transition", trace_id: sec.trace_id, message: Some(format!("from={} to=refunded", cur.state)) });
        }
        let rec = repo::transition_state(db, &req.id, repo::IntentState::Refunded).await
            .map_err(|e| ApiError::Internal { trace_id: sec.trace_id, message: Some(format!("db_error: {e}")) })?;
        if let Some(pi) = rec { return Ok(Json(IntentResponse { id: pi.id, state: pi.state })); }
        return Err(ApiError::NotFound { code: "payment_intent_not_found", trace_id: sec.trace_id });
    }
    Ok(Json(IntentResponse { id: req.id, state: "refunded".into() }))
}

#[derive(Deserialize)]
pub struct GetPath { pub id: String }

pub async fn get_intent(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    _headers: HeaderMap,
    axum::extract::Path(GetPath { id }): axum::extract::Path<GetPath>,
) -> Result<Json<IntentResponse>, ApiError> {
    if ensure_capability(&sec, Capability::PaymentProcess).is_err() {
        #[cfg(any(feature = "kafka", feature = "kafka-producer"))] emit_capability_denial_audit(state.audit_producer.as_deref(), &sec, Capability::PaymentProcess, "payment-service").await;
        return Err(ApiError::ForbiddenMissingRole { role: "payment_access", trace_id: sec.trace_id });
    }
    if let Some(db) = &state.db {
        let rec = repo::get_intent(db, &id).await
            .map_err(|e| ApiError::Internal { trace_id: sec.trace_id, message: Some(format!("db_error: {e}")) })?;
    if let Some(pi) = rec { return Ok(Json(IntentResponse { id: pi.id, state: pi.state })); }
    return Err(ApiError::NotFound { code: "payment_intent_not_found", trace_id: sec.trace_id });
    }
    // Fallback behavior without DB: pretend created
    Ok(Json(IntentResponse { id, state: "created".into() }))
}

#[allow(unused_variables)]
pub async fn process_card_payment(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    _headers: HeaderMap,
    Json(req): Json<PaymentRequest>,
) -> Result<Json<PaymentResponse>, ApiError> {
    if ensure_capability(&sec, Capability::PaymentProcess).is_err() {
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] emit_capability_denial_audit(state.audit_producer.as_deref(), &sec, Capability::PaymentProcess, "payment-service").await;
        return Err(ApiError::ForbiddenMissingRole { role: "payment_access", trace_id: sec.trace_id });
    }
    let _tenant_id = sec.tenant_id;

    let amount_money = Money::new(req.amount.clone());
    println!("Valor stub: processing card payment for Order {} amount={} (normalized={})", req.order_id, req.amount, amount_money);
    sleep(Duration::from_secs(2)).await;
    let approval_code = format!(
        "VAL-APPROVED-{}",
        &req.order_id[..8.min(req.order_id.len())]
    );
    println!("Valor stub: payment approved, code={}", approval_code);
    let response = PaymentResponse {
        status: "approved".into(),
        approval_code,
    };
    Ok(Json(response))
}

#[allow(unused_variables)]
pub async fn void_card_payment(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    _headers: HeaderMap,
    Json(req): Json<VoidPaymentRequest>,
) -> Result<Json<VoidPaymentResponse>, ApiError> {
    if ensure_capability(&sec, Capability::PaymentProcess).is_err() {
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] emit_capability_denial_audit(state.audit_producer.as_deref(), &sec, Capability::PaymentProcess, "payment-service").await;
        return Err(ApiError::ForbiddenMissingRole { role: "payment_access", trace_id: sec.trace_id });
    }
    let _tenant_id = sec.tenant_id;

    let amount_money = Money::new(req.amount.clone());
    println!("Valor stub: voiding payment for Order {} amount={} (normalized={}) reason={:?}", req.order_id, req.amount, amount_money, req.reason);
    sleep(Duration::from_secs(1)).await;
    let approval_code = format!("VAL-VOID-{}", &req.order_id[..8.min(req.order_id.len())]);
    println!("Valor stub: payment voided, code={}", approval_code);
    let response = VoidPaymentResponse {
        status: "voided".into(),
        approval_code,
    };
    Ok(Json(response))
}
