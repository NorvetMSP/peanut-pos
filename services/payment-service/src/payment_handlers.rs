use crate::{AppState, PAYMENT_ROLES};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use common_auth::{ensure_role, tenant_id_from_request, AuthContext};
use serde::{Deserialize, Serialize};
use bigdecimal::BigDecimal;
use common_money::normalize_scale;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Deserialize)]
pub struct PaymentRequest {
    #[serde(rename = "orderId")]
    pub order_id: String,
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
    auth: AuthContext,
    headers: HeaderMap,
    Json(req): Json<PaymentRequest>,
) -> Result<Json<PaymentResponse>, (StatusCode, String)> {
    ensure_role(&auth, PAYMENT_ROLES)?;
    let _tenant_id = tenant_id_from_request(&headers, &auth)?;

    println!("Valor stub: processing card payment for Order {} amount={} (normalized={})", req.order_id, req.amount, normalize_scale(&req.amount));
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
    auth: AuthContext,
    headers: HeaderMap,
    Json(req): Json<VoidPaymentRequest>,
) -> Result<Json<VoidPaymentResponse>, (StatusCode, String)> {
    ensure_role(&auth, PAYMENT_ROLES)?;
    let _tenant_id = tenant_id_from_request(&headers, &auth)?;

    println!("Valor stub: voiding payment for Order {} amount={} (normalized={}) reason={:?}", req.order_id, req.amount, normalize_scale(&req.amount), req.reason);
    sleep(Duration::from_secs(1)).await;
    let approval_code = format!("VAL-VOID-{}", &req.order_id[..8.min(req.order_id.len())]);
    println!("Valor stub: payment voided, code={}", approval_code);
    let response = VoidPaymentResponse {
        status: "voided".into(),
        approval_code,
    };
    Ok(Json(response))
}
