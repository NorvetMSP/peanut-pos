use crate::{AppState, PAYMENT_ROLES};
use axum::{
    extract::State,
    http::HeaderMap,
    Json,
};
use common_security::{SecurityCtxExtractor, roles::ensure_any_role};
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

pub async fn process_card_payment(
    State(_state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    _headers: HeaderMap,
    Json(req): Json<PaymentRequest>,
) -> Result<Json<PaymentResponse>, ApiError> {
    if ensure_any_role(&sec, PAYMENT_ROLES).is_err() {
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

pub async fn void_card_payment(
    State(_state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    _headers: HeaderMap,
    Json(req): Json<VoidPaymentRequest>,
) -> Result<Json<VoidPaymentResponse>, ApiError> {
    if ensure_any_role(&sec, PAYMENT_ROLES).is_err() {
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
