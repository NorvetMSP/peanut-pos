use axum::{Json, http::{HeaderMap, StatusCode}};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::AppState;
use reqwest::Client;
use rdkafka::producer::FutureRecord;
use sqlx::query_as;

#[derive(Deserialize)]
pub struct OrderItem {
    pub product_id: Uuid,
    pub quantity: i32
}

#[derive(Deserialize)]
pub struct NewOrder {
    pub items: Vec<OrderItem>,
    pub payment_method: String,
    pub total: f64
}

#[derive(Serialize, Debug)]
pub struct Order {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub total: f64,
    pub status: String
}

pub async fn create_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(new_order): Json<NewOrder>
) -> Result<Json<Order>, (StatusCode, String)> {
    // Extract tenant ID from headers (multi-tenant context)
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => return Err((StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID header".to_string()))
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Missing X-Tenant-ID header".to_string()));
    };
    // Generate a new order ID
    let order_id = Uuid::new_v4();
    // Call Integration Gateway to process payment
    let gateway_url = std::env::var("INTEGRATION_URL").unwrap_or_else(|_| "http://localhost:8083".to_string());
    let client = Client::new();
    let resp = client.post(format!("{}/payments", gateway_url))
        .json(&serde_json::json!({ "orderId": order_id.to_string(), "method": new_order.payment_method, "amount": new_order.total }))
        .send().await;
    if let Err(err) = resp {
        return Err((StatusCode::BAD_GATEWAY, format!("Payment request failed: {}", err)));
    }
    let resp = resp.unwrap();
    if !resp.status().is_success() {
        return Err((StatusCode::BAD_GATEWAY, "Payment was declined".to_string()));
    }
    // Payment successful, insert order into database
    let order = query_as!(
        Order,
        "INSERT INTO orders (id, tenant_id, total, status) VALUES ($1, $2, $3, $4) RETURNING id, tenant_id, total, status",
        order_id,
        tenant_id,
        new_order.total,
        "COMPLETED"
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;
    // Publish order.completed event to Kafka
    let event = serde_json::json!({
        "order_id": order.id,
        "tenant_id": tenant_id,
        "items": new_order.items,
        "total": new_order.total
    });
    let payload = event.to_string();
    let produce_future = state.kafka_producer.send(
        FutureRecord::to("order.completed").payload(&payload).key(&tenant_id.to_string()),
        0
    );
    if let Err(e) = produce_future.await {
        eprintln!("Failed to send order.completed event: {:?}", e);
    }
    // Return the created order
    Ok(Json(order))
}

pub async fn list_orders(
    State(state): State<AppState>,
    headers: HeaderMap
) -> Result<Json<Vec<Order>>, (StatusCode, String)> {
    // Extract tenant ID from headers
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => return Err((StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID header".to_string()))
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Missing X-Tenant-ID header".to_string()));
    };
    // Fetch orders for this tenant from database
    let orders = query_as!(
        Order,
        "SELECT id, tenant_id, total, status FROM orders WHERE tenant_id = $1",
        tenant_id
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;
    Ok(Json(orders))
}
