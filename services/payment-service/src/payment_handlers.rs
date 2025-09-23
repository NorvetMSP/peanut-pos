use crate::{ensure_role, tenant_id_from_request, AppState, PAYMENT_ROLES};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use common_auth::AuthContext;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

#[derive(Deserialize)]
pub struct PaymentRequest {
    #[serde(rename = "orderId")]
    pub order_id: String,
    pub method: String,
    pub amount: f64,
}

#[derive(Serialize)]
pub struct PaymentResponse {
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

    println!(
        "Valor stub: processing card payment for Order {}",
        req.order_id
    );
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
