use axum::{Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use crate::AppState;
use rdkafka::producer::FutureRecord;
use tokio::time::{sleep, Duration};

#[derive(Deserialize)]
pub struct PaymentRequest {
    #[serde(rename = "orderId")]
    pub order_id: String,
    pub method: String,
    pub amount: f64
}

#[derive(Serialize)]
pub struct PaymentResult {
    pub status: String
}

pub async fn process_payment(
    State(state): State<AppState>,
    Json(req): Json<PaymentRequest>
) -> Result<Json<PaymentResult>, (StatusCode, String)> {
    // Simulate processing (no actual external calls in prototype)
    // Publish payment.completed event
    let event = serde_json::json!({ "order_id": req.order_id, "method": req.method });
    let payload = event.to_string();
    if let Err(e) = state.kafka_producer.send(
        FutureRecord::to("payment.completed").payload(&payload).key(&req.order_id),
        0
    ).await {
        eprintln!("Failed to send payment.completed event: {:?}", e);
    }
    // Return simulated success response
    let result = PaymentResult { status: "paid".to_string() };
    Ok(Json(result))
}
