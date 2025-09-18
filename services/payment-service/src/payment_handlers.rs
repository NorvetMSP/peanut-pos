use axum::Json;
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

pub async fn process_card_payment(Json(req): Json<PaymentRequest>) -> Json<PaymentResponse> {
    println!(
        "Valor stub: processing card payment for Order {}",
        req.order_id
    );
    // Simulate interaction with Valor terminal (e.g., waiting for customer to tap card)
    sleep(Duration::from_secs(2)).await;
    // Generate a fake approval code or token
    let approval_code = format!("VAL-APPROVED-{}", &req.order_id[..8]);
    println!("Valor stub: payment approved, code={}", approval_code);
    let response = PaymentResponse {
        status: "approved".into(),
        approval_code,
    };
    Json(response)
}
