use axum::extract::State;
use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};
use rdkafka::producer::FutureRecord;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

use crate::AppState;

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
    let tenant_id = headers
        .get("X-Tenant-ID")
        .and_then(|hdr| hdr.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing or invalid X-Tenant-ID header".to_string(),
        ))?;

    let order_id = Uuid::new_v4();
    let NewOrder {
        items,
        payment_method,
        total,
    } = new_order;

    let status = if payment_method.eq_ignore_ascii_case("crypto") {
        "PENDING"
    } else {
        "COMPLETED"
    };

    let order = sqlx::query_as::<_, Order>(
        "INSERT INTO orders (id, tenant_id, total, status) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id, tenant_id, total, status",
    )
    .bind(order_id)
    .bind(tenant_id)
    .bind(total)
    .bind(status)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("DB error: {}", e),
        )
    })?;

    if status == "COMPLETED" {
        let event = serde_json::json!({
            "order_id": order.id,
            "tenant_id": tenant_id,
            "items": items,
            "total": total
        });
        if let Err(err) = state
            .kafka_producer
            .send(
                FutureRecord::to("order.completed")
                    .payload(&event.to_string())
                    .key(&tenant_id.to_string()),
                Duration::from_secs(0),
            )
            .await
        {
            tracing::error!("Failed to send order.completed: {:?}", err);
        }
    } else if let Some(pending_orders) = &state.pending_orders {
        pending_orders.lock().unwrap().insert(order.id, items);
    }

    Ok(Json(order))
}

pub async fn list_orders(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Order>>, (StatusCode, String)> {
    let tenant_id = headers
        .get("X-Tenant-ID")
        .and_then(|hdr| hdr.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing or invalid X-Tenant-ID header".to_string(),
        ))?;

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
