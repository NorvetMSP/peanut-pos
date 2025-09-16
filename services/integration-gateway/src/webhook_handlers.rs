use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use bytes::Bytes;
use hmac::{Hmac, Mac};
use rdkafka::producer::FutureRecord;
use serde::Deserialize;
use sha2::Sha256;
use std::time::Duration;
use uuid::Uuid;

use crate::{events::PaymentCompletedEvent, AppState};

#[derive(Deserialize)]
struct CoinbaseWebhook {
    event: CoinbaseEvent,
}

#[derive(Deserialize)]
struct CoinbaseEvent {
    #[serde(rename = "type")]
    event_type: String,
    data: CoinbaseCharge,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct CoinbaseCharge {
    id: String,
    code: String,
    hosted_url: Option<String>,
    metadata: Option<CoinbaseMeta>,
    pricing: Option<CoinbasePricing>, // not used for now
}

#[derive(Deserialize)]
struct CoinbaseMeta {
    order_id: Option<String>,
    tenant_id: Option<String>,
    amount: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct CoinbasePricing {
    local: Option<PriceInfo>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct PriceInfo {
    amount: String,
    currency: String,
}

pub async fn handle_coinbase_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let sig_header = headers
        .get("X-CC-Webhook-Signature")
        .and_then(|h| h.to_str().ok());
    let secret = std::env::var("COINBASE_WEBHOOK_SECRET").unwrap_or_default();
    if sig_header.is_none() || secret.is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    let signature = sig_header.unwrap();
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can be created");
    mac.update(&body);
    let calc_sig = hex::encode(mac.finalize().into_bytes());
    if calc_sig != signature {
        tracing::warn!("Coinbase webhook signature mismatch");
        return StatusCode::UNAUTHORIZED;
    }

    let Ok(webhook) = serde_json::from_slice::<CoinbaseWebhook>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    if webhook.event.event_type == "charge:confirmed" {
        if let Some(meta) = webhook.event.data.metadata {
            if let (Some(order_id_str), Some(tenant_id_str), Some(amount_str)) =
                (meta.order_id, meta.tenant_id, meta.amount)
            {
                if let (Ok(order_id), Ok(tenant_id), Ok(amount)) = (
                    Uuid::parse_str(&order_id_str),
                    Uuid::parse_str(&tenant_id_str),
                    amount_str.parse::<f64>(),
                ) {
                    let pay_event = PaymentCompletedEvent {
                        order_id,
                        tenant_id,
                        method: "crypto".to_string(),
                        amount,
                    };
                    let payload = serde_json::to_string(&pay_event).unwrap();
                    if let Err(err) = state
                        .kafka_producer
                        .send(
                            FutureRecord::to("payment.completed")
                                .payload(&payload)
                                .key(&tenant_id_str),
                            Duration::from_secs(0),
                        )
                        .await
                    {
                        tracing::error!("Failed to emit payment.completed: {:?}", err);
                    } else {
                        tracing::info!(
                            "Emitted payment.completed for order {} (crypto confirmed)",
                            order_id
                        );
                    }
                }
            }
        }
    }

    StatusCode::OK
}
