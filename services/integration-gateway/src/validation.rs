//! Future order/payment validation (gated by `future-order-validation` feature).
//! Provides lightweight existence/state checks prior to emitting a payment.voided event.
//!
//! Design notes:
//! - We avoid pulling full order/payment aggregates; instead perform a HEAD/GET lightweight check.
//! - Fail fast with domain-specific error strings which map to ApiError BadRequest in handler.
//! - Network calls kept minimal; any non-200 (except 404) escalates to internal error once
//!   richer error typing is introduced. For now we surface a generic validation failure message.
//! - This module intentionally has no external dependencies beyond reqwest + serde + uuid to keep
//!   feature compile overhead low.

use uuid::Uuid;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("order_not_found")] 
    OrderNotFound,
    #[error("payment_not_found")] 
    PaymentNotFound,
    #[error("payment_already_finalized")] 
    PaymentFinalized,
    #[error("http_error: {0}")] 
    Http(String),
}

/// Represents a minimal payment state response (subset of real payment service schema).
#[derive(serde::Deserialize)]
struct PaymentStateDto {
    status: String,
}

/// Validate a void request: order exists, payment exists, and payment is voidable (e.g. pending/authorized).
pub async fn validate_void_request(order_id: Uuid, tenant_id: Uuid, method: &str) -> Result<(), ValidationError> {
    // ORDER existence check
    if !fetch_order_exists(order_id, tenant_id).await? { return Err(ValidationError::OrderNotFound); }

    // PAYMENT existence + state check
    let payment = fetch_payment(order_id, tenant_id, method).await?;
    match payment.status.as_str() {
        "pending" | "authorized" => Ok(()),
        _ => Err(ValidationError::PaymentFinalized)
    }
}

async fn fetch_order_exists(order_id: Uuid, tenant_id: Uuid) -> Result<bool, ValidationError> {
    let base = std::env::var("ORDER_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8084".into());
    let url = format!("{}/orders/{}", base, order_id);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("X-Tenant-ID", tenant_id.to_string())
        .send().await.map_err(|e| ValidationError::Http(e.to_string()))?;
    match resp.status().as_u16() {
        200 => Ok(true),
        404 => Ok(false),
        s => Err(ValidationError::Http(format!("unexpected status {} while fetching order", s)))
    }
}

async fn fetch_payment(order_id: Uuid, tenant_id: Uuid, method: &str) -> Result<PaymentStateDto, ValidationError> {
    let base = std::env::var("PAYMENT_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8085".into());
    let url = format!("{}/payments/{}/{}", base, method, order_id);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("X-Tenant-ID", tenant_id.to_string())
        .send().await.map_err(|e| ValidationError::Http(e.to_string()))?;
    match resp.status().as_u16() {
        200 => resp.json::<PaymentStateDto>().await.map_err(|e| ValidationError::Http(e.to_string())),
        404 => Err(ValidationError::PaymentNotFound),
        s => Err(ValidationError::Http(format!("unexpected status {} while fetching payment", s)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn order_missing() {
        let server = MockServer::start();
        let order_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        // order 404
        server.mock(|when, then| { when.method(GET).path(format!("/orders/{}", order_id)); then.status(404); });
        std::env::set_var("ORDER_SERVICE_URL", server.base_url());
        let res = fetch_order_exists(order_id, tenant_id).await.unwrap();
        assert!(!res);
    }
}
