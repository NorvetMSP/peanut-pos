use axum::extract::State;
use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};
use common_auth::AuthContext;
use rdkafka::producer::FutureRecord;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

use crate::AppState;

const ORDER_WRITE_ROLES: &[&str] = &["super_admin", "admin", "manager", "cashier"];
const ORDER_REFUND_ROLES: &[&str] = &["super_admin", "admin", "manager"];
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
    pub customer_id: Option<String>,
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

pub async fn create_order(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Json(new_order): Json<NewOrder>,
) -> Result<Json<Order>, (StatusCode, String)> {
    ensure_role(&auth, ORDER_WRITE_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

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

    let customer_uuid = customer_id.as_ref().and_then(|value| {
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

    let order = sqlx::query_as::<_, Order>(
        "INSERT INTO orders (id, tenant_id, total, status, customer_id, offline)          VALUES ($1, $2, $3, $4, $5, $6)          RETURNING id, tenant_id, total::FLOAT8 as total, status, customer_id, offline",
    )
    .bind(order_id)
    .bind(tenant_id)
    .bind(total)
    .bind(status)
    .bind(customer_uuid)
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
            "customer_id": customer_uuid,
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
            .insert(order.id, (items, customer_uuid, offline_flag));
    }

    Ok(Json(order))
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
         RETURNING id, tenant_id, total::FLOAT8 as total, status, customer_id, offline",
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
    auth: AuthContext,
    headers: HeaderMap,
) -> Result<Json<Vec<Order>>, (StatusCode, String)> {
    ensure_role(&auth, ORDER_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let orders = sqlx::query_as::<_, Order>(
        "SELECT id, tenant_id, total::FLOAT8 as total, status, customer_id, offline          FROM orders          WHERE tenant_id = $1          ORDER BY created_at DESC          LIMIT 20",
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
