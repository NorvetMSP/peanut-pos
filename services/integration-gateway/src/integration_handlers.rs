use crate::AppState;

pub use crate::webhook_handlers::handle_coinbase_webhook;
use serde_json::json;
use axum::http::StatusCode;
use crate::AppState;
use serde::Serialize;
use serde::Deserialize;
use reqwest::Client;
use rdkafka::producer::FutureRecord;
use axum::extract::State;
use axum::Json;
use uuid::Uuid;

// Payment request/response types
#[derive(Deserialize)]
pub struct PaymentRequest {
    #[serde(rename = "orderId")]
    pub order_id: String,
    pub method: String,
    pub amount: f64,
}
#[derive(Serialize)]
pub struct PaymentResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_url: Option<String>,
}

pub async fn process_payment(
    State(state): State<AppState>,
    axum::Extension(tenant_id): axum::Extension<Uuid>,
    Json(req): Json<PaymentRequest>,
) -> Result<Json<PaymentResult>, (StatusCode, String)> {
    if req.method.eq_ignore_ascii_case("crypto") {
        // ...existing crypto branch...
        let api_key = std::env::var("COINBASE_COMMERCE_API_KEY")
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Coinbase API key not configured".to_string()))?;
        let client = Client::new();
        let charge_req = json!({
            "name": format!("Order {}", req.order_id),
            "description": "Cryptocurrency Payment",
            "local_price": {
                "amount": format!("{:.2}", req.amount),
                "currency": "USD"
            },
            "pricing_type": "fixed_price",
            "metadata": {
                "order_id": req.order_id,
                "tenant_id": tenant_id.to_string(),
                "amount": format!("{:.2}", req.amount)
            },
        });
        let api_resp = client
            .post("https://api.commerce.coinbase.com/charges")
            .header("X-CC-Api-Key", &api_key)
            .header("Content-Type", "application/json")
            .json(&charge_req)
            .send()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Coinbase API request failed: {}", e)))?;
        if !api_resp.status().is_success() {
            return Err((StatusCode::BAD_GATEWAY, "Failed to create crypto charge".into()));
        }
        let resp_body: serde_json::Value = api_resp.json().await
            .map_err(|_| (StatusCode::BAD_GATEWAY, "Invalid Coinbase response".into()))?;
        let hosted_url = resp_body.get("data")
            .and_then(|data| data.get("hosted_url"))
            .and_then(|url| url.as_str())
            .unwrap_or("");
        tracing::info!("Created Coinbase charge for order {} – Hosted URL: {}", req.order_id, hosted_url);
        let result = PaymentResult { status: "pending".into(), payment_url: Some(hosted_url.into()) };
        return Ok(Json(result));
    } else if req.method.eq_ignore_ascii_case("card") {
        // Forward card payment to Valor Payment Service
        let pay_svc_url = std::env::var("PAYMENT_SERVICE_URL")
            .unwrap_or_else(|_| "http://localhost:8086".to_string());
        let client = Client::new();
        let pay_resp = client
            .post(format!("{}/payments", pay_svc_url))
            .json(&req)
            .send()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Payment service error: {}", e)))?;
        if !pay_resp.status().is_success() {
            return Err((StatusCode::BAD_GATEWAY, "Payment was declined".into()));
        }
        // Parse response (e.g., get approval code, though we don't use it further in this stub)
        #[derive(Deserialize)]
        struct PaymentResponse { status: String, approval_code: String }
        let _approval = pay_resp.json::<PaymentResponse>().await
            .unwrap_or(PaymentResponse { status: "approved".into(), approval_code: "".into() });
        // Publish payment.completed event for the successful card payment
        let order_id = Uuid::parse_str(&req.order_id).unwrap_or(Uuid::nil());
        let pay_event = PaymentCompletedEvent {
            order_id,
            tenant_id,
            method: "card".to_string(),
            amount: req.amount,
        };
        let payload = serde_json::to_string(&pay_event).unwrap();
        if let Err(err) = state.kafka_producer.send(
            FutureRecord::to("payment.completed")
                .payload(&payload)
                .key(&tenant_id.to_string()),
            0,
        ).await {
            tracing::error!("Failed to send payment.completed: {:?}", err);
        }
        // Return success to caller
        let result = PaymentResult { status: "paid".into(), payment_url: None };
        return Ok(Json(result));
    } else {
        // Handle other payment methods (e.g., "cash") immediately
        let order_id = Uuid::parse_str(&req.order_id).unwrap_or(Uuid::nil());
        let pay_event = PaymentCompletedEvent {
            order_id,
            tenant_id,
            method: req.method.clone(),
            amount: req.amount,
        };
        let payload = serde_json::to_string(&pay_event).unwrap();
        let _ = state.kafka_producer.send(
            FutureRecord::to("payment.completed")
                .payload(&payload)
                .key(&tenant_id.to_string()),
            0,
        ).await;
        return Ok(Json(PaymentResult { status: "paid".into(), payment_url: None }));
    }
}
use axum::extract::State;
use axum::Json;
use reqwest::Client;
use axum::http::StatusCode;
use crate::AppState;
use uuid::Uuid;
use serde::Deserialize;

/// Order payload from external systems (mirrors Order Service's NewOrder format)
#[derive(Deserialize)]
pub struct ExternalOrder {
    pub items: Vec<OrderItem>,
    pub payment_method: String,
    pub total: f64,
}
#[derive(Deserialize)]
pub struct OrderItem {
    pub product_id: Uuid,
    pub quantity: i32,
}

/// Response type matching Order Service's Order struct
#[derive(Deserialize)]
struct Order {
    id: Uuid,
    tenant_id: Uuid,
    total: f64,
    status: String,
}

pub async fn handle_external_order(
    State(state): State<AppState>,
    axum::Extension(tenant_id): axum::Extension<Uuid>,
    Json(order): Json<ExternalOrder>,
) -> Result<Json<Order>, (StatusCode, String)> {
    // Forward the external order to the internal Order Service
    let order_svc_url = std::env::var("ORDER_SERVICE_URL")
        .unwrap_or_else(|_| "http://localhost:8084".to_string());
    let client = Client::new();
    let resp = client
        .post(format!("{}/orders", order_svc_url))
        .header("X-Tenant-ID", tenant_id.to_string())  // propagate tenant context
        .json(&order)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Order service request failed: {}", e)))?;
    if !resp.status().is_success() {
        // Propagate error from Order Service (e.g., validation failure or other)
        let code = if resp.status().is_client_error() { StatusCode::BAD_REQUEST } else { StatusCode::BAD_GATEWAY };
        let err_text = resp.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        return Err((code, format!("Order service error: {}", err_text)));
    }
    // Parse successful response as Order and return to external caller
    let created_order = resp.json::<Order>().await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Invalid response from Order Service: {}", e)))?;
    Ok(Json(created_order))
}
use axum::{http::StatusCode, Json};
use axum::extract::State;
use serde::{Deserialize, Serialize};
use crate::AppState;
use rdkafka::producer::FutureRecord;

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
    let event = serde_json::json!({ "order_id": req.order_id, "method": req.method, "amount": req.amount });
    let payload = event.to_string();
    if let Err(e) = state.kafka_producer.send(
        FutureRecord::to("payment.completed").payload(&payload).key(&req.order_id),
        None::<std::time::Duration>
    ).await {
        eprintln!("Failed to send payment.completed event: {:?}", e);
    }
    // Return simulated success response
    let result = PaymentResult { status: "paid".to_string() };
    Ok(Json(result))
}
