use axum::extract::{Path, Query, State};
use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};
use common_auth::AuthContext;
use rdkafka::producer::FutureRecord;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder, Row, Transaction};
use std::collections::HashMap;
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

#[derive(Serialize)]
struct InventoryReservationItemPayload {
    product_id: Uuid,
    quantity: i32,
}

#[allow(dead_code)]
#[derive(sqlx::FromRow)]
struct OrderStatusSnapshot {
    status: String,
    payment_method: String,
    customer_id: Option<Uuid>,
    offline: bool,
    total: Option<f64>,
}

#[derive(sqlx::FromRow)]
struct OrderItemFinancialRow {
    product_id: Uuid,
    quantity: i32,
    unit_price: f64,
    line_total: f64,
}

#[derive(Serialize)]
struct InventoryReservationRequestPayload {
    order_id: Uuid,
    items: Vec<InventoryReservationItemPayload>,
}

#[derive(Deserialize, Default)]
pub struct ListOrdersParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub status: Option<String>,
    pub customer_id: Option<Uuid>,
    pub order_id: Option<Uuid>,
    pub payment_method: Option<String>,
    pub q: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct OrderLineItem {
    pub product_id: Uuid,
    pub quantity: i32,
    pub unit_price: f64,
    pub line_total: f64,
    pub returned_quantity: i32,
}

#[derive(Serialize, Debug)]
pub struct OrderDetail {
    pub order: Order,
    pub items: Vec<OrderLineItem>,
}

#[derive(Serialize, Debug)]
pub struct ClearOfflineResponse {
    pub cleared: u64,
}

#[derive(Deserialize)]
pub struct RefundLine {
    pub product_id: Uuid,
    pub quantity: i32,
}

#[derive(Deserialize)]
pub struct RefundRequest {
    pub order_id: Uuid,
    pub items: Vec<RefundLine>,
    pub total: Option<f64>,
    pub reason: Option<String>,
}

#[derive(Deserialize)]
pub struct VoidOrderRequest {
    pub reason: Option<String>,
}

fn inventory_url(base_url: &str, suffix: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{}{}", trimmed, suffix)
}

async fn reserve_inventory(
    client: &Client,
    base_url: &str,
    tenant_id: Uuid,
    auth_token: &str,
    order_id: Uuid,
    items: &[OrderItem],
) -> Result<(), (StatusCode, String)> {
    let payload = InventoryReservationRequestPayload {
        order_id,
        items: items
            .iter()
            .map(|item| InventoryReservationItemPayload {
                product_id: item.product_id,
                quantity: item.quantity,
            })
            .collect(),
    };

    let mut request = client
        .post(inventory_url(base_url, "/inventory/reservations"))
        .header("Content-Type", "application/json")
        .header("X-Tenant-ID", tenant_id.to_string())
        .json(&payload);

    if !auth_token.is_empty() {
        request = request.header("Authorization", format!("Bearer {}", auth_token));
    }

    let response = request.send().await.map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Failed to contact inventory-service: {err}"),
        )
    })?;

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_else(|_| String::from(""));

    if status == reqwest::StatusCode::CONFLICT || status == reqwest::StatusCode::BAD_REQUEST {
        let mapped = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        return Err((
            mapped,
            if body.is_empty() {
                mapped.to_string()
            } else {
                body
            },
        ));
    }

    Err((
        StatusCode::BAD_GATEWAY,
        if body.is_empty() {
            format!("Inventory reservation failed with status {status}")
        } else {
            format!("Inventory reservation failed with status {status}: {body}")
        },
    ))
}

async fn release_inventory(
    client: &Client,
    base_url: &str,
    tenant_id: Uuid,
    auth_token: &str,
    order_id: Uuid,
) -> Result<(), String> {
    let mut request = client
        .delete(inventory_url(
            base_url,
            &format!("/inventory/reservations/{}", order_id),
        ))
        .header("X-Tenant-ID", tenant_id.to_string());

    if !auth_token.is_empty() {
        let header_value = format!("Bearer {}", auth_token);
        request = request.header("Authorization", header_value);
    }

    let response = request.send().await.map_err(|err| err.to_string())?;

    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_else(|_| String::from(""));

    if body.is_empty() {
        Err(format!("Inventory release failed with status {status}"))
    } else {
        Err(format!(
            "Inventory release failed with status {status}: {body}"
        ))
    }
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
            "SELECT id, tenant_id, total::FLOAT8 AS total, status, customer_id, offline, payment_method, idempotency_key FROM orders WHERE tenant_id = $1 AND idempotency_key = $2"
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
    let auth_token = auth.token.clone();

    reserve_inventory(
        &state.http_client,
        &state.inventory_base_url,
        tenant_id,
        &auth_token,
        order_id,
        &new_order.items,
    )
    .await?;

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

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            if let Err(release_err) = release_inventory(
                &state.http_client,
                &state.inventory_base_url,
                tenant_id,
                &auth_token,
                order_id,
            )
            .await
            {
                tracing::error!(?release_err, order_id = %order_id, tenant_id = %tenant_id, "Failed to release inventory after begin failure");
            }
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to begin transaction: {}", e),
            ));
        }
    };

    let order = match sqlx::query_as::<_, Order>(
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
    .await {
        Ok(order) => order,
        Err(e) => {
            if let Err(release_err) = release_inventory(
                &state.http_client,
                &state.inventory_base_url,
                tenant_id,
                &auth_token,
                order_id,
            )
            .await
            {
                tracing::error!(?release_err, order_id = %order_id, tenant_id = %tenant_id, "Failed to release inventory after insert failure");
            }
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to insert order: {}", e),
            ));
        }
    };

    if let Err(err) = insert_order_items(&mut tx, order.id, &new_order.items).await {
        if let Err(release_err) = release_inventory(
            &state.http_client,
            &state.inventory_base_url,
            tenant_id,
            &auth_token,
            order_id,
        )
        .await
        {
            tracing::error!(?release_err, order_id = %order_id, tenant_id = %tenant_id, "Failed to release inventory after order item insertion failure");
        }
        return Err(err);
    }

    if let Err(e) = tx.commit().await {
        if let Err(release_err) = release_inventory(
            &state.http_client,
            &state.inventory_base_url,
            tenant_id,
            &auth_token,
            order_id,
        )
        .await
        {
            tracing::error!(?release_err, order_id = %order_id, tenant_id = %tenant_id, "Failed to release inventory after commit failure");
        }
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to commit transaction: {}", e),
        ));
    }

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

    let existing = sqlx::query_as::<_, OrderStatusSnapshot>(
        "SELECT status, payment_method, customer_id, offline, total::FLOAT8 AS total FROM orders WHERE id = $1 AND tenant_id = $2",
    )
    .bind(order_id)
    .bind(tenant_id)
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

    let item_rows = sqlx::query_as::<_, OrderItemFinancialRow>(
        "SELECT product_id, quantity, unit_price::FLOAT8 AS unit_price, line_total::FLOAT8 AS line_total FROM order_items WHERE order_id = $1",
    )
    .bind(order_id)
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

    if req.items.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Refund must include at least one item".to_string(),
        ));
    }

    let mut tx = state.db.begin().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to begin refund transaction: {}", e),
        )
    })?;

    let order_snapshot = sqlx::query_as::<_, OrderStatusSnapshot>(
        "SELECT status, payment_method, customer_id, offline, total::FLOAT8 AS total FROM orders WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(req.order_id)
    .bind(tenant_id)
    .fetch_optional(&mut tx)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load order for refund: {}", e),
        )
    })?
    .ok_or((StatusCode::NOT_FOUND, "Order not found".to_string()))?;

    match order_snapshot.status.as_str() {
        "PENDING" | "VOIDED" | "NOT_ACCEPTED" => {
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "Order in status '{}' cannot be refunded",
                    order_snapshot.status
                ),
            ));
        }
        _ => {}
    }

    let db_rows = sqlx::query(
        "SELECT id, product_id, quantity, returned_quantity, unit_price::FLOAT8 AS unit_price FROM order_items WHERE order_id = $1 FOR UPDATE"
    )
    .bind(req.order_id)
    .fetch_all(&mut tx)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load order items for refund: {}", e),
        )
    })?;

    if db_rows.is_empty() {
        return Err((
            StatusCode::CONFLICT,
            "Order has no items to refund".to_string(),
        ));
    }

    struct PendingUpdate {
        order_item_id: Uuid,
        product_id: Uuid,
        quantity: i32,
        unit_price: f64,
        line_total: f64,
    }

    struct DbItem {
        order_item_id: Uuid,
        quantity: i32,
        returned_quantity: i32,
        unit_price: f64,
    }

    let mut items_map: HashMap<Uuid, DbItem> = HashMap::new();
    for row in db_rows {
        let order_item_id: Uuid = row.try_get("id").map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read order item id: {}", e),
            )
        })?;
        let product_id: Uuid = row.try_get("product_id").map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read order item product: {}", e),
            )
        })?;
        let quantity: i32 = row.try_get("quantity").map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read order item quantity: {}", e),
            )
        })?;
        let returned_quantity: i32 = row.try_get("returned_quantity").unwrap_or(0);
        let unit_price: f64 = row.try_get("unit_price").map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read order item unit price: {}", e),
            )
        })?;

        items_map.insert(
            product_id,
            DbItem {
                order_item_id,
                quantity,
                returned_quantity,
                unit_price,
            },
        );
    }

    let mut updates: Vec<PendingUpdate> = Vec::new();
    let mut refund_total = 0.0f64;

    for request_item in req.items.iter() {
        if request_item.quantity <= 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                "Refund quantities must be positive".to_string(),
            ));
        }

        let entry = items_map.get_mut(&request_item.product_id).ok_or((
            StatusCode::BAD_REQUEST,
            format!(
                "Product {} is not part of the order",
                request_item.product_id
            ),
        ))?;

        let available = entry.quantity - entry.returned_quantity;
        if available <= 0 {
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "All units of product {} have already been returned",
                    request_item.product_id
                ),
            ));
        }

        if request_item.quantity > available {
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "Cannot return {} units of product {}; only {} remain",
                    request_item.quantity, request_item.product_id, available
                ),
            ));
        }

        entry.returned_quantity += request_item.quantity;
        let line_total = entry.unit_price * request_item.quantity as f64;
        refund_total += line_total;
        updates.push(PendingUpdate {
            order_item_id: entry.order_item_id,
            product_id: request_item.product_id,
            quantity: request_item.quantity,
            unit_price: entry.unit_price,
            line_total,
        });
    }

    if updates.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "No valid items provided for refund".to_string(),
        ));
    }

    if let Some(client_total) = req.total {
        if (client_total - refund_total).abs() > 0.01 {
            tracing::warn!(
                order_id = %req.order_id,
                tenant_id = %tenant_id,
                client_total,
                computed_total = refund_total,
                "Client-supplied refund total differs from server computation"
            );
        }
    }

    for update in &updates {
        sqlx::query(
            "UPDATE order_items SET returned_quantity = returned_quantity + $1 WHERE id = $2",
        )
        .bind(update.quantity)
        .bind(update.order_item_id)
        .execute(&mut tx)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to write return quantity: {}", e),
            )
        })?;
    }

    let reason_text = req.reason.as_ref().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let return_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO order_returns (id, order_id, tenant_id, total, reason) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(return_id)
    .bind(req.order_id)
    .bind(tenant_id)
    .bind(refund_total)
    .bind(reason_text.as_deref())
    .execute(&mut tx)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to record order return: {}", e),
        )
    })?;

    for update in &updates {
        sqlx::query(
            "INSERT INTO order_return_items (id, return_id, order_item_id, quantity, line_total) VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(Uuid::new_v4())
        .bind(return_id)
        .bind(update.order_item_id)
        .bind(update.quantity)
        .bind(update.line_total)
        .execute(&mut tx)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to record order return items: {}", e),
            )
        })?;
    }

    let all_returned = items_map
        .values()
        .all(|item| item.returned_quantity >= item.quantity);
    let new_status = if all_returned {
        "REFUNDED"
    } else {
        "PARTIAL_REFUNDED"
    };

    let updated_order = sqlx::query_as::<_, Order>(
        "UPDATE orders SET status = $3 WHERE id = $1 AND tenant_id = $2
         RETURNING id, tenant_id, total::FLOAT8 AS total, status, customer_id, offline, payment_method, idempotency_key"
    )
    .bind(req.order_id)
    .bind(tenant_id)
    .bind(new_status)
    .fetch_one(&mut tx)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to update order status: {}", e),
        )
    })?;

    tx.commit().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to commit refund transaction: {}", e),
        )
    })?;

    let event_items: Vec<serde_json::Value> = updates
        .iter()
        .map(|update| {
            serde_json::json!({
                "product_id": update.product_id,
                "quantity": -update.quantity,
                "unit_price": update.unit_price,
                "line_total": -update.line_total,
            })
        })
        .collect();

    let refund_event = serde_json::json!({
        "order_id": updated_order.id,
        "tenant_id": tenant_id,
        "items": event_items,
        "total": -refund_total,
        "customer_id": updated_order.customer_id,
        "offline": updated_order.offline,
        "payment_method": updated_order.payment_method,
        "return_id": return_id,
    });

    if let Err(err) = state
        .kafka_producer
        .send(
            FutureRecord::to("order.completed")
                .payload(&refund_event.to_string())
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
    Query(params): Query<ListOrdersParams>,
) -> Result<Json<Vec<Order>>, (StatusCode, String)> {
    ensure_role(&auth, ORDER_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let offset = params.offset.unwrap_or(0).max(0);

    let mut builder = QueryBuilder::new(
        "SELECT id, tenant_id, total::FLOAT8 AS total, status, customer_id, offline, payment_method, idempotency_key FROM orders WHERE tenant_id = "
    );
    builder.push_bind(tenant_id);

    if let Some(order_id) = params.order_id {
        builder.push(" AND id = ");
        builder.push_bind(order_id);
    }

    if let Some(status) = params
        .status
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        builder.push(" AND status = ");
        builder.push_bind(status.to_uppercase());
    }

    if let Some(customer_id) = params.customer_id {
        builder.push(" AND customer_id = ");
        builder.push_bind(customer_id);
    }

    if let Some(payment_method) = params
        .payment_method
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        builder.push(" AND payment_method = ");
        builder.push_bind(payment_method.to_lowercase());
    }

    if let Some(term) = params
        .q
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        let pattern = format!("%{}%", term);
        builder.push(" AND (CAST(id AS TEXT) ILIKE ");
        builder.push_bind(pattern.clone());
        builder.push(" OR COALESCE(customer_id::TEXT, '') ILIKE ");
        builder.push_bind(pattern);
        builder.push(")");
    }

    builder.push(" ORDER BY created_at DESC");
    builder.push(" LIMIT ");
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);

    let orders = builder
        .build_query_as::<Order>()
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

pub async fn get_order(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(order_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<OrderDetail>, (StatusCode, String)> {
    ensure_role(&auth, ORDER_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let order = sqlx::query_as::<_, Order>(
        "SELECT id, tenant_id, total::FLOAT8 AS total, status, customer_id, offline, payment_method, idempotency_key FROM orders WHERE id = $1 AND tenant_id = $2",
    )
    .bind(order_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load order: {}", e),
        )
    })?
    .ok_or((StatusCode::NOT_FOUND, "Order not found".to_string()))?;

    let item_rows = sqlx::query(
        "SELECT product_id, quantity, returned_quantity, unit_price::FLOAT8 AS unit_price, line_total::FLOAT8 AS line_total FROM order_items WHERE order_id = $1 ORDER BY created_at"
    )
    .bind(order_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load order items: {}", e),
        )
    })?;

    let items = item_rows
        .into_iter()
        .map(|row| {
            let product_id: Uuid = row.try_get("product_id").map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to read order item product: {}", e),
                )
            })?;
            let quantity: i32 = row.try_get("quantity").map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to read order item quantity: {}", e),
                )
            })?;
            let returned_quantity: i32 = row.try_get("returned_quantity").unwrap_or(0);
            let unit_price: f64 = row.try_get("unit_price").map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to read order item unit price: {}", e),
                )
            })?;
            let line_total: f64 = row.try_get("line_total").map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to read order item line total: {}", e),
                )
            })?;
            Ok(OrderLineItem {
                product_id,
                quantity,
                unit_price,
                line_total,
                returned_quantity,
            })
        })
        .collect::<Result<Vec<_>, (StatusCode, String)>>()?;

    Ok(Json(OrderDetail { order, items }))
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
