use axum::extract::{Path, State};
use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};
use common_auth::AuthContext;
use rdkafka::producer::FutureRecord;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::time::Duration;
use uuid::Uuid;

use crate::AppState;

const ORDER_WRITE_ROLES: &[&str] = &["super_admin", "admin", "manager", "cashier"];
const ORDER_REFUND_ROLES: &[&str] = &["super_admin", "admin", "manager"];
const ORDER_VOID_ROLES: &[&str] = &["super_admin", "admin", "manager"];
const ORDER_VIEW_ROLES: &[&str] = &["super_admin", "admin", "manager", "cashier"];

fn ensure_role(auth: &AuthContext, allowed: &[&str]) -> Result<(), (StatusCode, String)> {
    let has_role = auth
        .claims
        .roles
        .iter()
        .any(|role| allowed.iter().any(|required| role == required));
    if has_role {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            format!("Insufficient role. Required one of: {}", allowed.join(", ")),
        ))
    }
}

fn tenant_id_from_request(
    headers: &HeaderMap,
    auth: &AuthContext,
) -> Result<Uuid, (StatusCode, String)> {
    let header_value = headers
        .get("X-Tenant-ID")
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing X-Tenant-ID header".to_string(),
        ))?
        .to_str()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid X-Tenant-ID header".to_string(),
            )
        })?
        .trim();
    let tenant_id = Uuid::parse_str(header_value).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid X-Tenant-ID header".to_string(),
        )
    })?;
    if tenant_id != auth.claims.tenant_id {
        return Err((
            StatusCode::FORBIDDEN,
            "Authenticated tenant does not match X-Tenant-ID header".to_string(),
        ));
    }
    Ok(tenant_id)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct OrderItem {
    pub product_id: Uuid,
    pub quantity: i32,
    pub unit_price: f64,
    pub line_total: f64,
}

#[derive(Deserialize, Debug)]
pub struct NewOrder {
    pub items: Vec<OrderItem>,
    pub payment_method: String,
    pub total: f64,
    pub customer_id: Option<String>,
    pub offline: Option<bool>,
    pub idempotency_key: Option<String>,
}

#[derive(Serialize, Debug, sqlx::FromRow)]
pub struct Order {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub total: f64,
    pub status: String,
    pub customer_id: Option<Uuid>,
    pub offline: bool,
    pub payment_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct ClearOfflineResponse {
    pub cleared: u64,
}

#[derive(Deserialize)]
pub struct RefundRequest {
    pub order_id: Uuid,
    pub items: Vec<OrderItem>,
    pub total: f64,
}

#[derive(Deserialize)]
pub struct VoidOrderRequest {
    pub reason: Option<String>,
}

async fn notify_payment_void(
    order_id: Uuid,
    tenant_id: Uuid,
    payment_method: &str,
    amount: f64,
    reason: Option<&str>,
) -> Result<(), String> {
    if !payment_method.eq_ignore_ascii_case("card")
        && !payment_method.eq_ignore_ascii_case("crypto")
    {
        return Ok(());
    }

    let base_url = std::env::var("INTEGRATION_GATEWAY_URL")
        .unwrap_or_else(|_| "http://localhost:8083".to_string());
    let url = format!("{}/payments/void", base_url.trim_end_matches('/'));

    let mut body = serde_json::json!({
        "orderId": order_id.to_string(),
        "method": payment_method,
        "amount": amount,
    });
    if let Some(text) = reason {
        if !text.is_empty() {
            body["reason"] = serde_json::Value::String(text.to_string());
        }
    }

    let client = Client::new();
    let mut request = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("X-Tenant-ID", tenant_id.to_string())
        .json(&body);

    if let Ok(api_key) = std::env::var("INTEGRATION_GATEWAY_API_KEY") {
        if !api_key.is_empty() {
            request = request.header("X-API-Key", api_key);
        }
    }

    let response = request
        .send()
        .await
        .map_err(|err| format!("Failed to contact integration-gateway: {err}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response
            .text()
            .await
            .unwrap_or_else(|_| String::from("<no body>"));
        return Err(format!(
            "integration-gateway void request failed (status {status}): {text}"
        ));
    }

    Ok(())
}

async fn insert_order_items(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    items: &[OrderItem],
) -> Result<(), (StatusCode, String)> {
    for item in items {
        sqlx::query("INSERT INTO order_items (id, order_id, product_id, quantity, unit_price, line_total) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(Uuid::new_v4())
        .bind(order_id)
        .bind(item.product_id)
        .bind(item.quantity)
        .bind(item.unit_price)
        .bind(item.line_total)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to insert order item: {}", e),
            )
        })?;
    }
    Ok(())
}

pub async fn create_order(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Json(new_order): Json<NewOrder>,
) -> Result<Json<Order>, (StatusCode, String)> {
    ensure_role(&auth, ORDER_WRITE_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    if new_order.items.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Order must contain at least one item".to_string(),
        ));
    }

    let mut payment_method = new_order.payment_method.trim().to_lowercase();
    if payment_method.is_empty() {
        payment_method = "cash".to_string();
    }

    let idempotency_key = new_order
        .idempotency_key
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    if let Some(ref key) = idempotency_key {
        if let Some(existing) = sqlx::query_as::<_, Order>(
            "SELECT id, tenant_id, total::FLOAT8 AS total, status, customer_id, offline, payment_method, idempotency_key
             FROM orders WHERE tenant_id = $1 AND idempotency_key = $2"
        )
        .bind(tenant_id)
        .bind(key)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("DB error while checking idempotency: {}", e),
            )
        })?
        {
            return Ok(Json(existing));
        }
    }

    let total_from_items: f64 = new_order.items.iter().map(|item| item.line_total).sum();
    if (total_from_items - new_order.total).abs() > 0.01 {
        tracing::warn!(
            tenant_id = %tenant_id,
            provided_total = new_order.total,
            derived_total = total_from_items,
            "Order total differs from item aggregate by more than a cent"
        );
    }

    let order_id = Uuid::new_v4();
    let offline_flag = new_order.offline.unwrap_or(false);
    let status = if payment_method.eq_ignore_ascii_case("crypto")
        || payment_method.eq_ignore_ascii_case("card")
    {
        "PENDING"
    } else {
        "COMPLETED"
    };

    let customer_uuid = new_order.customer_id.as_ref().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            match Uuid::parse_str(trimmed) {
                Ok(uuid) => Some(uuid),
                Err(err) => {
                    tracing::warn!(
                        customer_id = trimmed,
                        ?err,
                        "Ignoring invalid customer_id; proceeding without association"
                    );
                    None
                }
            }
        }
    });

    let mut tx = state.db.begin().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to begin transaction: {}", e),
        )
    })?;

    let order = sqlx::query_as::<_, Order>(
        "INSERT INTO orders (id, tenant_id, total, status, customer_id, offline, payment_method, idempotency_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id, tenant_id, total::FLOAT8 AS total, status, customer_id, offline, payment_method, idempotency_key"
    )
    .bind(order_id)
    .bind(tenant_id)
    .bind(new_order.total)
    .bind(status)
    .bind(customer_uuid)
    .bind(offline_flag)
    .bind(&payment_method)
    .bind(idempotency_key.as_deref())
    .fetch_one(&mut tx)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to insert order: {}", e),
        )
    })?;

    insert_order_items(&mut tx, order.id, &new_order.items).await?;

    tx.commit().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to commit transaction: {}", e),
        )
    })?;

    if order.status == "COMPLETED" {
        let event_items: Vec<serde_json::Value> = new_order
            .items
            .iter()
            .map(|item| {
                serde_json::json!({
                    "product_id": item.product_id,
                    "quantity": item.quantity,
                    "unit_price": item.unit_price,
                    "line_total": item.line_total
                })
            })
            .collect();

        let event = serde_json::json!({
            "order_id": order.id,
            "tenant_id": tenant_id,
            "items": event_items,
            "total": new_order.total,
            "customer_id": customer_uuid,
            "offline": order.offline,
            "payment_method": order.payment_method,
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
    }

    Ok(Json(order))
}

pub async fn void_order(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(order_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<VoidOrderRequest>,
) -> Result<Json<Order>, (StatusCode, String)> {
    ensure_role(&auth, ORDER_VOID_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let void_reason = req
        .reason
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    let existing = sqlx::query!(
        "SELECT status, payment_method, customer_id, offline, total::FLOAT8 AS total FROM orders WHERE id = $1 AND tenant_id = $2",
        order_id,
        tenant_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to fetch order: {}", e),
        )
    })?;

    let existing = existing.ok_or((StatusCode::NOT_FOUND, "Order not found".to_string()))?;

    if existing.status.as_str() != "PENDING" {
        return Err((
            StatusCode::CONFLICT,
            format!("Order in status '{}' cannot be voided", existing.status),
        ));
    }

    if let Err(err) = notify_payment_void(
        order_id,
        tenant_id,
        &existing.payment_method,
        existing.total.unwrap_or(0.0),
        void_reason.as_deref(),
    )
    .await
    {
        tracing::error!(order_id = %order_id, tenant_id = %tenant_id, ?err, "Failed to notify payment void");
        return Err((
            StatusCode::BAD_GATEWAY,
            "Unable to void payment with upstream provider".to_string(),
        ));
    }

    let updated_order = sqlx::query_as::<_, Order>(
        "UPDATE orders SET status = 'VOIDED', void_reason = $3 WHERE id = $1 AND tenant_id = $2 AND status = 'PENDING'
         RETURNING id, tenant_id, total::FLOAT8 AS total, status, customer_id, offline, payment_method, idempotency_key",
    )
    .bind(order_id)
    .bind(tenant_id)
    .bind(void_reason.as_deref())
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to void order: {}", e),
        )
    })?
    .ok_or((
        StatusCode::CONFLICT,
        "Order status changed before void could be applied".to_string(),
    ))?;

    let item_rows = sqlx::query!(
        "SELECT product_id, quantity, unit_price::FLOAT8 AS unit_price, line_total::FLOAT8 AS line_total FROM order_items WHERE order_id = $1",
        order_id
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load order items: {}", e),
        )
    })?;

    let event_items: Vec<serde_json::Value> = item_rows
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "product_id": row.product_id,
                "quantity": row.quantity,
                "unit_price": row.unit_price,
                "line_total": row.line_total,
            })
        })
        .collect();

    let order_void_event = serde_json::json!({
        "order_id": updated_order.id,
        "tenant_id": tenant_id,
        "items": event_items,
        "total": updated_order.total,
        "customer_id": updated_order.customer_id,
        "offline": updated_order.offline,
        "payment_method": updated_order.payment_method,
        "reason": void_reason,
    });

    if let Err(err) = state
        .kafka_producer
        .send(
            FutureRecord::to("order.voided")
                .payload(&order_void_event.to_string())
                .key(&tenant_id.to_string()),
            Duration::from_secs(0),
        )
        .await
    {
        tracing::error!("Failed to send order.voided: {:?}", err);
    }

    Ok(Json(updated_order))
}

pub async fn refund_order(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Json(req): Json<RefundRequest>,
) -> Result<Json<Order>, (StatusCode, String)> {
    ensure_role(&auth, ORDER_REFUND_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let updated_order = sqlx::query_as::<_, Order>(
        "UPDATE orders SET status = 'REFUNDED' WHERE id = $1 AND tenant_id = $2
         RETURNING id, tenant_id, total::FLOAT8 as total, status, customer_id, offline, payment_method, idempotency_key",
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
                "quantity": -(item.quantity as i32),
                "unit_price": item.unit_price,
                "line_total": -item.line_total,
            })
        })
        .collect();
    let event = serde_json::json!({
        "order_id": req.order_id,
        "tenant_id": tenant_id,
        "items": neg_items,
        "total": -req.total,
        "customer_id": updated_order.customer_id,
        "offline": updated_order.offline,
        "payment_method": updated_order.payment_method,
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
    auth: AuthContext,
    headers: HeaderMap,
) -> Result<Json<Vec<Order>>, (StatusCode, String)> {
    ensure_role(&auth, ORDER_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let orders = sqlx::query_as::<_, Order>(
        "SELECT id, tenant_id, total::FLOAT8 as total, status, customer_id, offline, payment_method, idempotency_key
         FROM orders
         WHERE tenant_id = $1
         ORDER BY created_at DESC
         LIMIT 20",
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

pub async fn clear_offline_orders(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
) -> Result<Json<ClearOfflineResponse>, (StatusCode, String)> {
    ensure_role(&auth, ORDER_REFUND_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let result = sqlx::query("DELETE FROM orders WHERE tenant_id = $1 AND offline = TRUE")
        .bind(tenant_id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to clear offline orders: {}", e),
            )
        })?;

    Ok(Json(ClearOfflineResponse {
        cleared: result.rows_affected(),
    }))
}
