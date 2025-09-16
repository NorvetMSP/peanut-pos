use axum::extract::{Extension, State};
use axum::{http::StatusCode, Json};
use rdkafka::producer::FutureRecord;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

use crate::{events::PaymentCompletedEvent, AppState};

// Payment request/response types
#[derive(Deserialize, Serialize)]
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

#[allow(dead_code)]
#[derive(Deserialize)]
struct PaymentServiceResponse {
    status: String,
    approval_code: String,
}

pub async fn process_payment(
    State(state): State<AppState>,
    Extension(tenant_id): Extension<Uuid>,
    Json(req): Json<PaymentRequest>,
) -> Result<Json<PaymentResult>, (StatusCode, String)> {
    if req.method.eq_ignore_ascii_case("crypto") {
        let api_key = std::env::var("COINBASE_COMMERCE_API_KEY").map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Coinbase API key not configured".to_string(),
            )
        })?;
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
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    format!("Coinbase API request failed: {}", e),
                )
            })?;
        if !api_resp.status().is_success() {
            return Err((
                StatusCode::BAD_GATEWAY,
                "Failed to create crypto charge".into(),
            ));
        }
        let resp_body: serde_json::Value = api_resp
            .json()
            .await
            .map_err(|_| (StatusCode::BAD_GATEWAY, "Invalid Coinbase response".into()))?;
        let hosted_url = resp_body
            .get("data")
            .and_then(|data| data.get("hosted_url"))
            .and_then(|url| url.as_str())
            .unwrap_or("");
        tracing::info!(
            "Created Coinbase charge for order {} - Hosted URL: {}",
            req.order_id,
            hosted_url
        );
        let result = PaymentResult {
            status: "pending".into(),
            payment_url: Some(hosted_url.into()),
        };
        return Ok(Json(result));
    }

    if req.method.eq_ignore_ascii_case("card") {
        let pay_svc_url = std::env::var("PAYMENT_SERVICE_URL")
            .unwrap_or_else(|_| "http://localhost:8086".to_string());
        let client = Client::new();
        let pay_resp = client
            .post(format!("{}/payments", pay_svc_url))
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    format!("Payment service error: {}", e),
                )
            })?;
        if !pay_resp.status().is_success() {
            return Err((StatusCode::BAD_GATEWAY, "Payment was declined".into()));
        }
        let _approval =
            pay_resp
                .json::<PaymentServiceResponse>()
                .await
                .unwrap_or(PaymentServiceResponse {
                    status: "approved".into(),
                    approval_code: String::new(),
                });
    }

    let order_id = Uuid::parse_str(&req.order_id).unwrap_or_else(|_| Uuid::nil());
    let pay_event = PaymentCompletedEvent {
        order_id,
        tenant_id,
        method: req.method.clone(),
        amount: req.amount,
    };
    let payload = serde_json::to_string(&pay_event).unwrap();
    if let Err(err) = state
        .kafka_producer
        .send(
            FutureRecord::to("payment.completed")
                .payload(&payload)
                .key(&tenant_id.to_string()),
            Duration::from_secs(0),
        )
        .await
    {
        tracing::error!("Failed to send payment.completed: {:?}", err);
        return Err((
            StatusCode::BAD_GATEWAY,
            "Failed to notify payment completion".into(),
        ));
    }

    Ok(Json(PaymentResult {
        status: "paid".into(),
        payment_url: None,
    }))
}

/// Order payload from external systems (mirrors Order Service's NewOrder format)
#[derive(Deserialize, Serialize)]
pub struct ExternalOrder {
    pub items: Vec<OrderItem>,
    pub payment_method: String,
    pub total: f64,
}

#[derive(Deserialize, Serialize)]
pub struct OrderItem {
    pub product_id: Uuid,
    pub quantity: i32,
}

/// Response type matching Order Service's Order struct
#[derive(Deserialize, Serialize)]
pub(crate) struct Order {
    id: Uuid,
    tenant_id: Uuid,
    total: f64,
    status: String,
}

pub async fn handle_external_order(
    Extension(tenant_id): Extension<Uuid>,
    Json(order): Json<ExternalOrder>,
) -> Result<Json<Order>, (StatusCode, String)> {
    let order_svc_url =
        std::env::var("ORDER_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8084".to_string());
    let client = Client::new();
    let resp = client
        .post(format!("{}/orders", order_svc_url))
        .header("X-Tenant-ID", tenant_id.to_string())
        .json(&order)
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Order service request failed: {}", e),
            )
        })?;
    if !resp.status().is_success() {
        let code = if resp.status().is_client_error() {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::BAD_GATEWAY
        };
        let err_text = resp
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err((code, format!("Order service error: {}", err_text)));
    }

    let created_order = resp.json::<Order>().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Invalid response from Order Service: {}", e),
        )
    })?;
    Ok(Json(created_order))
}
