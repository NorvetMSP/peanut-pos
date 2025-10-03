use axum::extract::{Extension, State};
use axum::Json;
use common_http_errors::{ApiError, ApiResult};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::{FutureProducer, FutureRecord};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

#[allow(unused_imports)]
use crate::{
    events::{PaymentCompletedEvent, PaymentVoidedEvent},
    AppState,
};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use crate::events::PaymentFailedEvent;

#[derive(Clone)]
pub struct ForwardedAuthHeader(pub String);

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

use common_security::SecurityCtxExtractor;

pub async fn process_payment(
    #[allow(unused_variables)] State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    forwarded_auth: Option<Extension<ForwardedAuthHeader>>,
    Json(req): Json<PaymentRequest>,
) -> ApiResult<Json<PaymentResult>> {
    let tenant_id = sec.tenant_id;
    let order_id = Uuid::parse_str(&req.order_id).map_err(|_| ApiError::BadRequest {
        code: "invalid_order_id",
        trace_id: None,
        message: Some("Invalid orderId".into()),
    })?;

    // --- Crypto payment flow ---
    if req.method.eq_ignore_ascii_case("crypto") {
        let mut use_stub = std::env::var("COINBASE_STUB_MODE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let api_key = std::env::var("COINBASE_COMMERCE_API_KEY").ok();
        if api_key.is_none() && !use_stub {
            tracing::warn!("COINBASE_COMMERCE_API_KEY missing; falling back to stub mode");
            use_stub = true;
        }

        if use_stub {
            let hosted_url = format!("https://stub.coinbase.local/charges/{}", Uuid::new_v4());
            #[allow(unused_variables)] let amount = req.amount;
            #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
            {
                let producer = state.kafka_producer.clone();
                let tenant_key = tenant_id.to_string();
                tokio::spawn(async move {
                    sleep(Duration::from_secs(5)).await;
                    let completion = PaymentCompletedEvent { order_id, tenant_id, method: "crypto".into(), amount };
                    if let Ok(payload) = serde_json::to_string(&completion) {
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
                });
            }
            #[cfg(not(any(feature = "kafka", feature = "kafka-producer")))]
            {
                tokio::spawn(async move {
                    sleep(Duration::from_secs(5)).await;
                    tracing::info!(order_id = %order_id, "Stub crypto payment completed (no kafka)");
                });
            }
            return Ok(Json(PaymentResult { status: "pending".into(), payment_url: Some(hosted_url) }));
        }

        let api_key = api_key.expect("API key must exist when not using stub");
        let client = Client::new();
        let charge_req = json!({
            "name": format!("Order {}", req.order_id),
            "description": "Cryptocurrency Payment",
            "local_price": { "amount": format!("{:.2}", req.amount), "currency": "USD" },
            "pricing_type": "fixed_price",
            "metadata": { "order_id": req.order_id, "tenant_id": tenant_id.to_string(), "amount": format!("{:.2}", req.amount) }
        });
        let api_resp = match client
            .post("https://api.commerce.coinbase.com/charges")
            .header("X-CC-Api-Key", &api_key)
            .header("Content-Type", "application/json")
            .json(&charge_req)
            .send()
            .await
        {
            Ok(r) => r,
            Err(err) => {
                let reason = format!("Coinbase API request failed: {err}");
                #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
                emit_payment_failed(&state.kafka_producer, tenant_id, order_id, &req.method, &reason).await;
                return Err(ApiError::Internal { trace_id: None, message: Some(reason) });
            }
        };
        if !api_resp.status().is_success() {
            let _reason = format!("Coinbase charge rejected with status {}", api_resp.status());
            #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
            {
                let reason = format!("Coinbase charge rejected with status {}", api_resp.status());
                emit_payment_failed(&state.kafka_producer, tenant_id, order_id, &req.method, &reason).await;
            }
            return Err(ApiError::Internal { trace_id: None, message: Some("Failed to create crypto charge".into()) });
        }
        let body: serde_json::Value = match api_resp.json().await {
            Ok(b) => b,
            Err(_err) => {
                #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
                {
                    let reason = "Invalid Coinbase response".to_string();
                    emit_payment_failed(&state.kafka_producer, tenant_id, order_id, &req.method, &reason).await;
                }
                return Err(ApiError::Internal { trace_id: None, message: Some("Invalid Coinbase response".into()) });
            }
        };
        let hosted_url = body
            .get("data")
            .and_then(|d| d.get("hosted_url"))
            .and_then(|u| u.as_str())
            .unwrap_or("");
        if hosted_url.is_empty() {
            #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
            {
                let reason = "Coinbase response missing hosted_url".to_string();
                emit_payment_failed(&state.kafka_producer, tenant_id, order_id, &req.method, &reason).await;
            }
            return Err(ApiError::Internal { trace_id: None, message: Some("Failed to create crypto charge".into()) });
        }
        tracing::info!(order_id = %order_id, hosted_url, "Created Coinbase charge");
        return Ok(Json(PaymentResult { status: "pending".into(), payment_url: Some(hosted_url.to_string()) }));
    }

    // --- Card payment flow ---
    if req.method.eq_ignore_ascii_case("card") {
        let pay_svc_url = std::env::var("PAYMENT_SERVICE_URL")
            .unwrap_or_else(|_| "http://localhost:8086".to_string());
        let client = Client::new();
        let mut pay_request = client
            .post(format!("{}/payments", pay_svc_url.trim_end_matches('/')))
            .header("Content-Type", "application/json")
            .header("X-Tenant-ID", tenant_id.to_string())
            .json(&req);
        if let Some(Extension(auth)) = forwarded_auth.as_ref() {
            pay_request = pay_request.header("Authorization", auth.0.as_str());
        }
        let pay_resp = match pay_request.send().await {
            Ok(r) => r,
            Err(err) => {
                #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
                {
                    let reason = format!("Payment service error: {err}");
                    emit_payment_failed(&state.kafka_producer, tenant_id, order_id, &req.method, &reason).await;
                }
                return Err(ApiError::Internal { trace_id: None, message: Some(format!("Payment service error: {err}")) });
            }
        };
        if !pay_resp.status().is_success() {
            #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
            {
                let reason = format!("Payment declined (status {})", pay_resp.status());
                emit_payment_failed(&state.kafka_producer, tenant_id, order_id, &req.method, &reason).await;
            }
            return Err(ApiError::Internal { trace_id: None, message: Some("Payment was declined".into()) });
        }
        // Optionally parse approval; ignore errors (non-fatal)
        let _ = pay_resp.json::<PaymentServiceResponse>().await;
    }

    // Emit completion event for non-crypto (immediate) or card payments
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    {
        let pay_event = PaymentCompletedEvent {
            order_id,
            tenant_id,
            method: req.method.clone(),
            amount: req.amount,
        };
        if let Ok(payload) = serde_json::to_string(&pay_event) {
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
                tracing::error!(?err, order_id = %order_id, "Failed to send payment.completed");
                return Err(ApiError::Internal { trace_id: None, message: Some("Failed to notify payment completion".into()) });
            }
        }
    }

    Ok(Json(PaymentResult { status: "paid".into(), payment_url: None }))
}

#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
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

// When kafka features are disabled, emit_payment_failed is never invoked.

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
pub struct Order {
    id: Uuid,
    tenant_id: Uuid,
    total: f64,
    status: String,
}

pub async fn handle_external_order(
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Json(order): Json<ExternalOrder>,
) -> ApiResult<Json<Order>> {
    let tenant_id = sec.tenant_id;
    let order_svc_url =
        std::env::var("ORDER_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8084".to_string());
    let client = Client::new();
    let resp = client
        .post(format!("{}/orders", order_svc_url))
        .header("X-Tenant-ID", tenant_id.to_string())
        .json(&order)
        .send()
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Order service request failed: {}", e)) })?;
    if !resp.status().is_success() {
        let code = if resp.status().is_client_error() {
            ApiError::BadRequest { code: "order_service_error", trace_id: None, message: Some(format!("Order service error status {}", resp.status())) }
        } else {
            ApiError::Internal { trace_id: None, message: Some(format!("Order service error status {}", resp.status())) }
        };
        return Err(code);
    }

    let created_order = resp.json::<Order>().await.map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Invalid response from Order Service: {}", e)) })?;
    Ok(Json(created_order))
}

/// Void a previously pending payment (best-effort demo implementation).
/// For now this only emits a PaymentVoidedEvent (when Kafka is enabled) and returns a synthetic status.
#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
pub async fn void_payment(
    #[allow(unused_variables)] State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Json(req): Json<PaymentVoidRequest>,
) -> ApiResult<Json<PaymentResult>> {
    let order_id = Uuid::parse_str(&req.order_id)
        .map_err(|_| ApiError::BadRequest { code: "invalid_order_id", trace_id: None, message: Some("Invalid orderId".into()) })?;
    let tenant_id = sec.tenant_id;
    use tracing::info;
    // Placeholder domain validation hook (order/payment existence, state checks) - future real integration.
    #[cfg(feature = "future-order-validation")]
    if let Err(err) = crate::validation::validate_void_request(order_id, tenant_id, &req.method).await {
        return Err(ApiError::BadRequest { code: "void_validation_failed", trace_id: None, message: Some(err.to_string()) });
    }

    let event = PaymentVoidedEvent {
        order_id,
        tenant_id,
        method: req.method.clone(),
        amount: req.amount,
        reason: req.reason.clone(),
    };
    if let Ok(payload) = serde_json::to_string(&event) {
        if std::env::var("TEST_CAPTURE_KAFKA").ok().as_deref() == Some("1") {
            test_support::capture_payment_voided(&payload);
        }
        // Test shortcut: allow skipping actual broker send when running in capture mode without a Kafka cluster.
        if std::env::var("TEST_KAFKA_NO_BROKER").ok().as_deref() == Some("1") {
            info!(order_id = %order_id, "Skipped broker send (TEST_KAFKA_NO_BROKER=1)");
            return Ok(Json(PaymentResult { status: "voided".into(), payment_url: None }));
        }
        if let Err(err) = state.kafka_producer
            .send(
                FutureRecord::to("payment.voided")
                    .payload(&payload)
                    .key(&tenant_id.to_string()),
                Duration::from_secs(0),
            )
            .await
        {
            tracing::error!(?err, order_id = %order_id, "Failed to emit payment.voided");
            return Err(ApiError::Internal { trace_id: None, message: Some("Failed to emit void event".into()) });
        } else {
            info!(order_id = %order_id, "Emitted payment.voided event");
        }
    }
    Ok(Json(PaymentResult { status: "voided".into(), payment_url: None }))
}

#[cfg(not(any(feature = "kafka", feature = "kafka-producer")))]
pub async fn void_payment(
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Json(req): Json<PaymentVoidRequest>,
) -> ApiResult<Json<PaymentResult>> {
    let _tenant_id = sec.tenant_id;
    let order_id = Uuid::parse_str(&req.order_id)
        .map_err(|_| ApiError::BadRequest { code: "invalid_order_id", trace_id: None, message: Some("Invalid orderId".into()) })?;
    let _ = (&order_id, &req.method);
    // Non-kafka build: validation hook could still be wired later if needed.
    Ok(Json(PaymentResult { status: "voided".into(), payment_url: None }))
}

// Test support submodule exposing capture drain helpers (available when tests compile the crate)
#[cfg(any(test, feature = "kafka", feature = "kafka-producer"))]
pub mod test_support {
    use std::sync::{Mutex, OnceLock};

    static CAPTURED_PAYMENT_VOIDED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    static CAPTURED_RATE_LIMIT_ALERTS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

    pub fn capture_payment_voided(payload: &str) {
        let store = CAPTURED_PAYMENT_VOIDED.get_or_init(|| Mutex::new(Vec::new()));
        store.lock().unwrap().push(payload.to_string());
    }

    pub fn take_captured_payment_voided() -> Vec<String> {
        let store = CAPTURED_PAYMENT_VOIDED.get_or_init(|| Mutex::new(Vec::new()));
        let mut guard = store.lock().unwrap();
        let drained = guard.drain(..).collect();
        drained
    }

    pub fn capture_rate_limit_alert(payload: &str) {
        let store = CAPTURED_RATE_LIMIT_ALERTS.get_or_init(|| Mutex::new(Vec::new()));
        store.lock().unwrap().push(payload.to_string());
    }

    pub fn take_captured_rate_limit_alerts() -> Vec<String> {
        let store = CAPTURED_RATE_LIMIT_ALERTS.get_or_init(|| Mutex::new(Vec::new()));
        let mut guard = store.lock().unwrap();
        guard.drain(..).collect()
    }
}
