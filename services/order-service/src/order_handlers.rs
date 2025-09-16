use axum::extract::State;
use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use rdkafka::producer::FutureRecord;
use reqwest::Client;

#[derive(Deserialize, Serialize, Debug)]
pub struct OrderItem {
    pub product_id: Uuid,
    pub quantity: i32,
}

#[derive(Deserialize, Debug)]
pub struct NewOrder {
    pub items: Vec<OrderItem>,
    pub payment_method: String,
    pub total: f64,
}

#[derive(Serialize, Debug, sqlx::FromRow)]
pub struct Order {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub total: f64,
    pub status: String,
}

pub async fn create_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(new_order): Json<NewOrder>,
) -> Result<Json<Order>, (StatusCode, String)> {
    // Extract tenant ID from headers (multi-tenant context)
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Invalid X-Tenant-ID header".to_string(),
                ))
            }
        }
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing X-Tenant-ID header".to_string(),
        ));
    };

    let order_id = Uuid::new_v4();
    let NewOrder {
        items,
        payment_method,
        total,
    } = new_order;

    // Call Integration Gateway to process payment
    let gateway_url =
        std::env::var("INTEGRATION_URL").unwrap_or_else(|_| "http://localhost:8083".to_string());
    let client = Client::new();
    let resp = client
        .post(format!("{}/payments", gateway_url))
        .json(&serde_json::json!({
            "orderId": order_id.to_string(),
            "method": payment_method,
            "amount": total,
        }))
        .send()
        .await;

    if let Err(err) = resp {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("Payment request failed: {}", err),
        ));
    }

    let resp = resp.unwrap();
    if !resp.status().is_success() {
        return Err((StatusCode::BAD_GATEWAY, "Payment was declined".to_string()));
    }

    // Payment successful, insert order into database
    let order = sqlx::query_as::<_, Order>(
        "INSERT INTO orders (id, tenant_id, total, status) VALUES ($1, $2, $3, $4) RETURNING id, tenant_id, total, status",
    )
    .bind(order_id)
    .bind(tenant_id)
    .bind(total)
    .bind("COMPLETED")
    .fetch_one(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;

    // Publish order.completed event to Kafka
    let event = serde_json::json!({
        "order_id": order.id,
        "tenant_id": tenant_id,
        "items": items,
        "total": total,
    });
    let payload = event.to_string();
    use std::time::Duration;
    let tenant_id_str = tenant_id.to_string();
    let produce_future = state.kafka_producer.send(
        FutureRecord::to("order.completed")
            .payload(&payload)
            .key(&tenant_id_str),
        Duration::from_secs(0),
    );

    if let Err(e) = produce_future.await {
        eprintln!("Failed to send order.completed event: {:?}", e);
    }

    Ok(Json(order))
}

pub async fn list_orders(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Order>>, (StatusCode, String)> {
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Invalid X-Tenant-ID header".to_string(),
                ))
            }
        }
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing X-Tenant-ID header".to_string(),
        ));
    };

    let orders = sqlx::query_as::<_, Order>(
        "SELECT id, tenant_id, total, status FROM orders WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {}", e),
        )
    })?;

    Ok(Json(orders))
}
