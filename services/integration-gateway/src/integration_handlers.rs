use axum::extract::{Extension, State};
use axum::{http::StatusCode, Json};
use rdkafka::producer::{FutureProducer, FutureRecord};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

use crate::{
    events::{PaymentCompletedEvent, PaymentFailedEvent, PaymentVoidedEvent},
    AppState,
};

// Payment request/response types
#[derive(Deserialize, Serialize)]
pub struct PaymentRequest {
    #[serde(rename = "orderId")]
    pub order_id: String,
    pub method: String,
    pub amount: f64,
}

#[derive(Deserialize, Serialize)]
pub struct PaymentVoidRequest {
    #[serde(rename = "orderId")]
    pub order_id: String,
    pub method: String,
    pub amount: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
    let order_id = Uuid::parse_str(&req.order_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid orderId".to_string()))?;

    if req.method.eq_ignore_ascii_case("crypto") {
        let mut use_stub = std::env::var("COINBASE_STUB_MODE")
            .map(|val| val.eq_ignore_ascii_case("true") || val == "1")
            .unwrap_or(false);
        let api_key = std::env::var("COINBASE_COMMERCE_API_KEY").ok();
        if api_key.is_none() && !use_stub {
            tracing::warn!("COINBASE_COMMERCE_API_KEY missing; using Coinbase stub mode");
            use_stub = true;
        }

        if use_stub {
            let hosted_url = format!("https://stub.coinbase.local/charges/{}", Uuid::new_v4());
            tracing::info!(order_id = %order_id, "Generated stub Coinbase charge");
            let producer = state.kafka_producer.clone();
            let tenant_key = tenant_id.to_string();
            let amount = req.amount;
            tokio::spawn(async move {
                sleep(Duration::from_secs(5)).await;
                let completion = PaymentCompletedEvent {
                    order_id,
                    tenant_id,
                    method: "crypto".to_string(),
                    amount,
                };
                match serde_json::to_string(&completion) {
                    Ok(payload) => {
                        if let Err(err) = producer
                            .send(
                                FutureRecord::to("payment.completed")
                                    .payload(&payload)
                                    .key(&tenant_key),
                                Duration::from_secs(0),
                            )
                            .await
                        {
                            tracing::error!(?err, order_id = %order_id, "Failed to emit stub payment.completed");
                        } else {
                            tracing::info!(order_id = %order_id, "Stub crypto payment auto-confirmed");
                        }
                    }
                    Err(err) => tracing::error!(
                        ?err,
                        order_id = %order_id,
                        "Failed to serialize stub payment.completed"
                    ),
                }
            });
            return Ok(Json(PaymentResult {
                status: "pending".into(),
                payment_url: Some(hosted_url),
            }));
        }

        let api_key = api_key.expect("API key must exist when not using stub");
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
        let api_resp = match client
            .post("https://api.commerce.coinbase.com/charges")
            .header("X-CC-Api-Key", &api_key)
            .header("Content-Type", "application/json")
            .json(&charge_req)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                let reason = format!("Coinbase API request failed: {err}");
                emit_payment_failed(
                    &state.kafka_producer,
                    tenant_id,
                    order_id,
                    &req.method,
                    &reason,
                )
                .await;
                return Err((StatusCode::BAD_GATEWAY, reason));
            }
        };
        if !api_resp.status().is_success() {
            let reason = format!("Coinbase charge rejected with status {}", api_resp.status());
            emit_payment_failed(
                &state.kafka_producer,
                tenant_id,
                order_id,
                &req.method,
                &reason,
            )
            .await;
            return Err((
                StatusCode::BAD_GATEWAY,
                "Failed to create crypto charge".into(),
            ));
        }
        let resp_body: serde_json::Value = match api_resp.json().await {
            Ok(body) => body,
            Err(err) => {
                let reason = format!("Invalid Coinbase response: {err}");
                emit_payment_failed(
                    &state.kafka_producer,
                    tenant_id,
                    order_id,
                    &req.method,
                    &reason,
                )
                .await;
                return Err((StatusCode::BAD_GATEWAY, "Invalid Coinbase response".into()));
            }
        };
        let hosted_url = resp_body
            .get("data")
            .and_then(|data| data.get("hosted_url"))
            .and_then(|url| url.as_str())
            .unwrap_or("");
        if hosted_url.is_empty() {
            let reason = "Coinbase response missing hosted_url".to_string();
            emit_payment_failed(
                &state.kafka_producer,
                tenant_id,
                order_id,
                &req.method,
                &reason,
            )
            .await;
            return Err((
                StatusCode::BAD_GATEWAY,
                "Failed to create crypto charge".into(),
            ));
        }
        tracing::info!(order_id = %order_id, hosted_url, "Created Coinbase charge");
        return Ok(Json(PaymentResult {
            status: "pending".into(),
            payment_url: Some(hosted_url.to_string()),
        }));
    }

    if req.method.eq_ignore_ascii_case("card") {
        let pay_svc_url = std::env::var("PAYMENT_SERVICE_URL")
            .unwrap_or_else(|_| "http://localhost:8086".to_string());
        let client = Client::new();
        let pay_resp = match client
            .post(format!("{}/payments", pay_svc_url))
            .json(&req)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                let reason = format!("Payment service error: {err}");
                emit_payment_failed(
                    &state.kafka_producer,
                    tenant_id,
                    order_id,
                    &req.method,
                    &reason,
                )
                .await;
                return Err((StatusCode::BAD_GATEWAY, reason));
            }
        };
        if !pay_resp.status().is_success() {
            let reason = format!("Payment declined (status {})", pay_resp.status());
            emit_payment_failed(
                &state.kafka_producer,
                tenant_id,
                order_id,
                &req.method,
                &reason,
            )
            .await;
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

pub async fn void_payment(
    State(state): State<AppState>,
    Extension(tenant_id): Extension<Uuid>,
    Json(req): Json<PaymentVoidRequest>,
) -> Result<Json<PaymentResult>, (StatusCode, String)> {
    let order_id = Uuid::parse_str(&req.order_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid orderId".to_string()))?;

    if req.method.eq_ignore_ascii_case("card") {
        let pay_svc_url = std::env::var("PAYMENT_SERVICE_URL")
            .unwrap_or_else(|_| "http://localhost:8086".to_string());
        let client = Client::new();
        let pay_resp = client
            .post(format!("{}/payments/void", pay_svc_url))
            .json(&req)
            .send()
            .await
            .map_err(|err| {
                let reason = format!("Payment service error: {err}");
                (StatusCode::BAD_GATEWAY, reason)
            })?;
        if !pay_resp.status().is_success() {
            let status = pay_resp.status();
            let reason = format!("Payment void declined (status {status})");
            return Err((StatusCode::BAD_GATEWAY, reason));
        }
    } else if req.method.eq_ignore_ascii_case("crypto") {
        tracing::info!(order_id = %order_id, "Stub crypto void processed");
    }

    let pay_event = PaymentVoidedEvent {
        order_id,
        tenant_id,
        method: req.method.clone(),
        amount: req.amount,
        reason: req.reason.clone(),
    };
    let payload = serde_json::to_string(&pay_event).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize payment.voided: {err}"),
        )
    })?;

    if let Err(err) = state
        .kafka_producer
        .send(
            FutureRecord::to("payment.voided")
                .payload(&payload)
                .key(&tenant_id.to_string()),
            Duration::from_secs(0),
        )
        .await
    {
        tracing::error!(?err, order_id = %order_id, "Failed to emit payment.voided");
    }

    Ok(Json(PaymentResult {
        status: "voided".into(),
        payment_url: None,
    }))
}

async fn emit_payment_failed(
    producer: &FutureProducer,
    tenant_id: Uuid,
    order_id: Uuid,
    method: &str,
    reason: &str,
) {
    let event = PaymentFailedEvent {
        order_id,
        tenant_id,
        method: method.to_string(),
        reason: reason.to_string(),
    };
    let payload = match serde_json::to_string(&event) {
        Ok(body) => body,
        Err(err) => {
            tracing::error!(?err, order_id = %order_id, "Failed to serialize payment.failed");
            return;
        }
    };
    if let Err(err) = producer
        .send(
            FutureRecord::to("payment.failed")
                .payload(&payload)
                .key(&tenant_id.to_string()),
            Duration::from_secs(0),
        )
        .await
    {
        tracing::error!(?err, order_id = %order_id, "Failed to send payment.failed");
    } else {
        tracing::info!(order_id = %order_id, method, reason, "Emitted payment.failed");
    }
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
