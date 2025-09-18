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
    pub customer_id: Option<Uuid>,
    pub offline: Option<bool>,
}

#[derive(Serialize, Debug, sqlx::FromRow)]
pub struct Order {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub total: f64,
    pub status: String,
    pub customer_id: Option<Uuid>,
    pub offline: bool,
}

#[derive(Deserialize)]
pub struct RefundRequest {
    pub order_id: Uuid,
    pub items: Vec<OrderItem>,
    pub total: f64,
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
        customer_id,
        offline,
    } = new_order;

    let offline_flag = offline.unwrap_or(false);

    let status = if payment_method.eq_ignore_ascii_case("crypto")
        || payment_method.eq_ignore_ascii_case("card")
    {
        "PENDING"
    } else {
        "COMPLETED"
    };

    let order = sqlx::query_as::<_, Order>(
        "INSERT INTO orders (id, tenant_id, total, status, customer_id, offline)          VALUES ($1, $2, $3, $4, $5, $6)          RETURNING id, tenant_id, total, status, customer_id, offline",
    )
    .bind(order_id)
    .bind(tenant_id)
    .bind(total)
    .bind(status)
    .bind(customer_id)
    .bind(offline_flag)
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
            "total": total,
            "customer_id": customer_id,
            "offline": order.offline
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
        pending_orders
            .lock()
            .unwrap()
            .insert(order.id, (items, customer_id, offline_flag));
    }

    Ok(Json(order))
}

pub async fn refund_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RefundRequest>,
) -> Result<Json<Order>, (StatusCode, String)> {
    let tenant_id = headers
        .get("X-Tenant-ID")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing or invalid X-Tenant-ID header".into(),
        ))?;

    let updated_order = sqlx::query_as::<_, Order>(
        "UPDATE orders SET status = 'REFUNDED' WHERE id = $1 AND tenant_id = $2
         RETURNING id, tenant_id, total, status, customer_id, offline",
    )
    .bind(req.order_id)
    .bind(tenant_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to update order: {}", e),
        )
    })?;

    let neg_items: Vec<serde_json::Value> = req
        .items
        .iter()
        .map(|item| {
            serde_json::json!({
                "product_id": item.product_id,
                "quantity": -(item.quantity as i32)
            })
        })
        .collect();
    let event = serde_json::json!({
        "order_id": req.order_id,
        "tenant_id": tenant_id,
        "items": neg_items,
        "total": -req.total,
        "customer_id": updated_order.customer_id,
        "offline": updated_order.offline
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
        tracing::error!("Failed to send order.completed (refund): {:?}", err);
    }

    Ok(Json(updated_order))
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
        "SELECT id, tenant_id, total, status, customer_id, offline          FROM orders          WHERE tenant_id = $1          ORDER BY created_at DESC          LIMIT 20",
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
