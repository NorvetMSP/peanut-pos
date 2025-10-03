use axum::extract::{Path, Query, State};
use axum::{
    http::{header::CONTENT_TYPE, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use common_auth::AuthContext; // retained only for access to bearer token for downstream service calls
use common_security::{SecurityCtxExtractor, Role};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use common_audit; // for AuditSeverity
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use serde_json::json;
use common_http_errors::ApiError;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::FutureRecord;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use bigdecimal::BigDecimal;
use common_money::{nearly_equal, Money};
// Removed unused FromStr import after BigDecimal migration
use sqlx::{Postgres, QueryBuilder, Row, Transaction};
use sqlx::Acquire; // acquire a connection handle within a transaction for sqlx 0.7 executor compatibility
use std::collections::HashMap;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use std::time::Duration;
use uuid::Uuid;

use crate::AppState;

// Legacy role string constants removed; unified role enforcement now via common-security Role enum.
// Mapping note: prior ROLE_CASHIER is approximated by Role::Support until a dedicated Cashier role is introduced.

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct OrderItem {
    pub product_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_name: Option<String>,
    pub quantity: i32,
    pub unit_price: Money,
    pub line_total: Money,
}

#[derive(Deserialize, Debug)]
pub struct NewOrder {
    pub items: Vec<OrderItem>,
    pub payment_method: String,
    pub total: BigDecimal, // accept raw, wrap later
    pub customer_id: Option<String>,
    pub customer_name: Option<String>,
    pub customer_email: Option<String>,
    pub store_id: Option<Uuid>,
    pub offline: Option<bool>,
    pub idempotency_key: Option<String>,
}

#[derive(Serialize, Debug, sqlx::FromRow)]
pub struct Order {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub total: Money,
    pub status: String,
    pub customer_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
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
    total: Option<BigDecimal>,
}

#[derive(sqlx::FromRow)]
struct OrderItemFinancialRow {
    product_id: Uuid,
    quantity: i32,
    unit_price: BigDecimal,
    line_total: BigDecimal,
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
    pub store_id: Option<Uuid>,
    pub customer: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub sort: Option<String>,
    pub direction: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ListReturnsParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub order_id: Option<Uuid>,
    pub store_id: Option<Uuid>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

#[derive(Serialize, Debug)]
pub struct ReturnSummary {
    pub id: Uuid,
    pub order_id: Uuid,
    pub total: Money,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_id: Option<Uuid>,
}
#[derive(Serialize, Debug)]
pub struct OrderLineItem {
    pub product_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_name: Option<String>,
    pub quantity: i32,
    pub unit_price: Money,
    pub line_total: Money,
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
    pub total: Option<BigDecimal>,
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

fn map_legacy_error(status: StatusCode, message: String) -> ApiError {
    match status {
        StatusCode::BAD_REQUEST => ApiError::BadRequest { code: "bad_request", trace_id: None, message: Some(message) },
        StatusCode::FORBIDDEN => ApiError::Forbidden { trace_id: None },
        StatusCode::NOT_FOUND => ApiError::NotFound { code: "not_found", trace_id: None },
        StatusCode::CONFLICT => ApiError::BadRequest { code: "conflict", trace_id: None, message: Some(message) },
        StatusCode::BAD_GATEWAY => ApiError::BadRequest { code: "upstream_error", trace_id: None, message: Some(message) },
        _ => ApiError::Internal { trace_id: None, message: Some(message) },
    }
}

async fn reserve_inventory(
    client: &Client,
    base_url: &str,
    tenant_id: Uuid,
    auth_token: &str,
    order_id: Uuid,
    items: &[OrderItem],
) -> Result<(), ApiError> {
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

    let response = request.send().await.map_err(|err| map_legacy_error(StatusCode::BAD_GATEWAY, format!("Failed to contact inventory-service: {err}")))?;

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_else(|_| String::from(""));

    if status == reqwest::StatusCode::CONFLICT || status == reqwest::StatusCode::BAD_REQUEST {
        let mapped = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    return Err(map_legacy_error(mapped, if body.is_empty() { mapped.to_string() } else { body }));
    }

    Err(map_legacy_error(StatusCode::BAD_GATEWAY,
        if body.is_empty() { format!("Inventory reservation failed with status {status}") } else { format!("Inventory reservation failed with status {status}: {body}") }
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
    amount: BigDecimal,
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
) -> Result<(), ApiError> {
    for item in items {
        let conn = tx
            .acquire()
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
            sqlx::query(
                r#"INSERT INTO order_items (id, order_id, product_id, quantity, unit_price, line_total, product_name)
                    VALUES ($1, $2, $3, $4, $5, $6, $7)"#
            )
            .bind(Uuid::new_v4())
            .bind(order_id)
            .bind(item.product_id)
            .bind(item.quantity)
            .bind(item.unit_price.inner())
            .bind(item.line_total.inner())
            .bind(item.product_name.as_deref())
            .execute(&mut *conn)
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to insert order item: {}", e)) })?;
    }
    Ok(())
}

pub async fn create_order(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    auth: AuthContext, // kept only for propagating bearer token to inventory-service
    Json(new_order): Json<NewOrder>,
) -> Result<Json<Order>, ApiError> {
    if !sec
        .roles
        .iter()
        .any(|r| matches!(r, Role::Admin | Role::Manager | Role::Support))
    {
    return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager_or_support", trace_id: None });
    }
    let tenant_id = sec.tenant_id;

    if new_order.items.is_empty() {
    return Err(ApiError::BadRequest { code: "missing_items", trace_id: None, message: Some("Order must contain at least one item".into()) });
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
            "SELECT id, tenant_id, total, status, customer_id, customer_name, customer_email, store_id, created_at, offline, payment_method, idempotency_key FROM orders WHERE tenant_id = $1 AND idempotency_key = $2"
        )
        .bind(tenant_id)
        .bind(key)
        .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("DB error while checking idempotency: {}", e)) })?
        {
            return Ok(Json(existing));
        }
    }

    let total_from_items: BigDecimal = new_order
        .items
        .iter()
    .fold(BigDecimal::from(0), |acc, item| acc + BigDecimal::from(item.line_total.clone())) ;
    if !nearly_equal(&total_from_items, &new_order.total, 1) {
        tracing::warn!(
            tenant_id = %tenant_id,
            provided_total = %new_order.total,
            derived_total = %total_from_items,
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

    let customer_name = new_order
        .customer_name
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    let customer_email = new_order
        .customer_email
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_lowercase());

    let store_id = new_order.store_id;

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
            return Err(ApiError::Internal { trace_id: None, message: Some(format!("Failed to begin transaction: {}", e)) });
        }
    };

    let order = match {
    let conn = tx
            .acquire()
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
        sqlx::query_as::<_, Order>(
            "INSERT INTO orders (id, tenant_id, total, status, customer_id, customer_name, customer_email, store_id, offline, payment_method, idempotency_key)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             RETURNING id, tenant_id, total, status, customer_id, customer_name, customer_email, store_id, created_at, offline, payment_method, idempotency_key"
        )
        .bind(order_id)
        .bind(tenant_id)
        .bind(&new_order.total)
        .bind(status)
        .bind(customer_uuid)
        .bind(customer_name.as_deref())
        .bind(customer_email.as_deref())
        .bind(store_id)
        .bind(offline_flag)
        .bind(&payment_method)
        .bind(idempotency_key.as_deref())
        .fetch_one(&mut *conn)
        .await
    } {
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
            return Err(ApiError::Internal { trace_id: None, message: Some(format!("Failed to insert order: {}", e)) });
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
        return Err(ApiError::Internal { trace_id: None, message: Some(format!("Failed to commit transaction: {}", e)) });
    }

    if order.status == "COMPLETED" {
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] let event_items: Vec<serde_json::Value> = new_order
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

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] let event = serde_json::json!({
            "order_id": order.id,
            "tenant_id": tenant_id,
            "items": event_items,
            "total": new_order.total,
            "customer_id": customer_uuid,
            "offline": order.offline,
            "payment_method": order.payment_method,
        });

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
        {
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
    }

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    if let Some(audit) = &state.audit_producer {
        let changes = json!({"after": {"order_id": order.id, "status": order.status, "total": order.total.inner()}});
        let _ = audit
            .emit(
                tenant_id,
                sec.actor.clone(),
                "order",
                Some(order.id),
                "created",
                "order-service",
                common_audit::AuditSeverity::Info,
                None,
                changes,
                json!({"source":"order-service"}),
            )
            .await;
    }
    Ok(Json(order))
}

pub async fn void_order(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(order_id): Path<Uuid>,
    Json(req): Json<VoidOrderRequest>,
) -> Result<Json<Order>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
    return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager", trace_id: None });
    }
    let tenant_id = sec.tenant_id;

    let void_reason = req
        .reason
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    let existing = sqlx::query_as::<_, OrderStatusSnapshot>(
    "SELECT status, payment_method, customer_id, offline, total FROM orders WHERE id = $1 AND tenant_id = $2",
    )
    .bind(order_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to fetch order: {}", e)) })?;

    let existing = existing.ok_or(ApiError::NotFound { code: "order_not_found", trace_id: None })?;

    if existing.status.as_str() != "PENDING" {
        return Err(ApiError::BadRequest { code: "invalid_status", trace_id: None, message: Some(format!("Order in status '{}' cannot be voided", existing.status)) });
    }

    if let Err(err) = notify_payment_void(
        order_id,
        tenant_id,
        &existing.payment_method,
        existing
            .total
            .clone()
            .unwrap_or_else(|| BigDecimal::from(0)),
        void_reason.as_deref(),
    )
    .await
    {
        tracing::error!(order_id = %order_id, tenant_id = %tenant_id, ?err, "Failed to notify payment void");
        return Err(ApiError::BadRequest { code: "upstream_error", trace_id: None, message: Some("Unable to void payment with upstream provider".into()) });
    }

    let updated_order = sqlx::query_as::<_, Order>(
        "UPDATE orders SET status = 'VOIDED', void_reason = $3 WHERE id = $1 AND tenant_id = $2 AND status = 'PENDING'
         RETURNING id, tenant_id, total, status, customer_id, customer_name, customer_email, store_id, created_at, offline, payment_method, idempotency_key",
    )
    .bind(order_id)
    .bind(tenant_id)
    .bind(void_reason.as_deref())
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to void order: {}", e)) })?
    .ok_or(ApiError::BadRequest { code: "status_changed", trace_id: None, message: Some("Order status changed before void could be applied".into()) })?;

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    let item_rows = sqlx::query_as::<_, OrderItemFinancialRow>(
        "SELECT product_id, quantity, unit_price, line_total FROM order_items WHERE order_id = $1",
    )
    .bind(order_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to load order items: {}", e)) })?;

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] let event_items: Vec<serde_json::Value> = item_rows
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

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] let order_void_event = serde_json::json!({
        "order_id": updated_order.id,
        "tenant_id": tenant_id,
        "items": event_items,
        "total": updated_order.total,
        "customer_id": updated_order.customer_id,
        "offline": updated_order.offline,
        "payment_method": updated_order.payment_method,
        "reason": void_reason,
    });

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
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

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    if let Some(audit) = &state.audit_producer {
        let changes = json!({
            "order_id": updated_order.id,
            "before": {"status": "PENDING"},
            "after": {"status": "VOIDED", "reason": void_reason},
        });
        let _ = audit
            .emit(
                tenant_id,
                sec.actor.clone(),
                "order",
                Some(updated_order.id),
                "voided",
                "order-service",
                common_audit::AuditSeverity::Info,
                None,
                changes,
                json!({"source":"order-service"}),
            )
            .await;
    }
    Ok(Json(updated_order))
}

pub async fn refund_order(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Json(req): Json<RefundRequest>,
) -> Result<Json<Order>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
    return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager", trace_id: None });
    }
    let tenant_id = sec.tenant_id;

    if req.items.is_empty() {
        return Err(ApiError::BadRequest { code: "missing_items", trace_id: None, message: Some("Refund must include at least one item".into()) });
    }

    let mut tx = state.db.begin().await.map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to begin refund transaction: {}", e)) })?;

    let order_snapshot = {
        let conn = tx
            .acquire()
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
        sqlx::query_as::<_, OrderStatusSnapshot>(
            "SELECT status, payment_method, customer_id, offline, total FROM orders WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(req.order_id)
        .bind(tenant_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to load order for refund: {}", e)) })?
        .ok_or(ApiError::NotFound { code: "order_not_found", trace_id: None })?
    };

    match order_snapshot.status.as_str() {
        "PENDING" | "VOIDED" | "NOT_ACCEPTED" => {
            return Err(ApiError::BadRequest { code: "invalid_status", trace_id: None, message: Some(format!("Order in status '{}' cannot be refunded", order_snapshot.status)) });
        }
        _ => {}
    }

    let db_rows = {
        let conn = tx
            .acquire()
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
        sqlx::query(
            "SELECT id, product_id, quantity, returned_quantity, unit_price FROM order_items WHERE order_id = $1 FOR UPDATE",
        )
        .bind(req.order_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to load order items for refund: {}", e)) })?
    };

    if db_rows.is_empty() {
        return Err(ApiError::BadRequest { code: "no_items", trace_id: None, message: Some("Order has no items to refund".into()) });
    }

    struct PendingUpdate {
        order_item_id: Uuid,
        product_id: Uuid,
        quantity: i32,
    unit_price: BigDecimal,
    line_total: BigDecimal,
    }

    struct DbItem {
        order_item_id: Uuid,
        quantity: i32,
        returned_quantity: i32,
    unit_price: BigDecimal,
    }

    let mut items_map: HashMap<Uuid, DbItem> = HashMap::new();
    for row in db_rows {
        let order_item_id: Uuid = row.try_get("id").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read order item id: {}", e)) })?;
        let product_id: Uuid = row.try_get("product_id").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read order item product: {}", e)) })?;
        let quantity: i32 = row.try_get("quantity").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read order item quantity: {}", e)) })?;
        let returned_quantity: i32 = row.try_get("returned_quantity").unwrap_or(0);
    let unit_price: BigDecimal = row.try_get("unit_price").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read order item unit price: {}", e)) })?;

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
    let mut refund_total = BigDecimal::from(0);

    for request_item in req.items.iter() {
        if request_item.quantity <= 0 {
            return Err(ApiError::BadRequest { code: "invalid_quantity", trace_id: None, message: Some("Refund quantities must be positive".into()) });
        }

        let entry = items_map.get_mut(&request_item.product_id).ok_or(ApiError::BadRequest { code: "product_not_in_order", trace_id: None, message: Some(format!("Product {} is not part of the order", request_item.product_id)) })?;

        let available = entry.quantity - entry.returned_quantity;
        if available <= 0 {
            return Err(ApiError::BadRequest { code: "already_returned", trace_id: None, message: Some(format!("All units of product {} have already been returned", request_item.product_id)) });
        }

        if request_item.quantity > available {
            return Err(ApiError::BadRequest { code: "exceeds_available", trace_id: None, message: Some(format!("Cannot return {} units of product {}; only {} remain", request_item.quantity, request_item.product_id, available)) });
        }

        entry.returned_quantity += request_item.quantity;
        let line_total = &entry.unit_price * BigDecimal::from(request_item.quantity);
        refund_total += line_total.clone();
        updates.push(PendingUpdate {
            order_item_id: entry.order_item_id,
            product_id: request_item.product_id,
            quantity: request_item.quantity,
            unit_price: entry.unit_price.clone(),
            line_total: line_total.clone(),
        });
    }

    if updates.is_empty() {
        return Err(ApiError::BadRequest { code: "no_valid_items", trace_id: None, message: Some("No valid items provided for refund".into()) });
    }

    if let Some(client_total) = req.total.clone() {
        if !nearly_equal(&client_total, &refund_total, 1) {
            tracing::warn!(
                order_id = %req.order_id,
                tenant_id = %tenant_id,
                client_total = %client_total,
                computed_total = %refund_total,
                "Client-supplied refund total differs from server computation"
            );
        }
    }

    for update in &updates {
        let conn = tx
            .acquire()
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
        sqlx::query(
            "UPDATE order_items SET returned_quantity = returned_quantity + $1 WHERE id = $2",
        )
        .bind(update.quantity)
        .bind(update.order_item_id)
        .execute(&mut *conn)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to write return quantity: {}", e)) })?;
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

    {
        let conn = tx
            .acquire()
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
        sqlx::query(
            "INSERT INTO order_returns (id, order_id, tenant_id, total, reason) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(return_id)
        .bind(req.order_id)
        .bind(tenant_id)
        .bind(&refund_total)
        .bind(reason_text.as_deref())
        .execute(&mut *conn)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to record order return: {}", e)) })?;
    }

    for update in &updates {
        let conn = tx
            .acquire()
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
        sqlx::query(
            "INSERT INTO order_return_items (id, return_id, order_item_id, quantity, line_total) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(return_id)
        .bind(update.order_item_id)
        .bind(update.quantity)
        .bind(&update.line_total)
        .execute(&mut *conn)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to record order return items: {}", e)) })?;
    }

    let all_returned = items_map
        .values()
        .all(|item| item.returned_quantity >= item.quantity);
    let new_status = if all_returned {
        "REFUNDED"
    } else {
        "PARTIAL_REFUNDED"
    };

    let updated_order = {
        let conn = tx
            .acquire()
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
        sqlx::query_as::<_, Order>(
            "UPDATE orders SET status = $3 WHERE id = $1 AND tenant_id = $2
             RETURNING id, tenant_id, total, status, customer_id, customer_name, customer_email, store_id, created_at, offline, payment_method, idempotency_key",
        )
        .bind(req.order_id)
        .bind(tenant_id)
        .bind(new_status)
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to update order status: {}", e)) })?
    };

    tx.commit().await.map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to commit refund transaction: {}", e)) })?;

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] let event_items: Vec<serde_json::Value> = updates
        .iter()
        .map(|update| {
            serde_json::json!({
                "product_id": update.product_id,
                "quantity": -update.quantity,
                "unit_price": update.unit_price,
                "line_total": update.line_total.clone() * BigDecimal::from(-1),
            })
        })
        .collect();

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] let refund_event = serde_json::json!({
        "order_id": updated_order.id,
        "tenant_id": tenant_id,
        "items": event_items,
    "total": (&refund_total * BigDecimal::from(-1)),
        "customer_id": updated_order.customer_id,
        "offline": updated_order.offline,
        "payment_method": updated_order.payment_method,
        "return_id": return_id,
    });

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
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

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    if let Some(audit) = &state.audit_producer {
        let changes = json!({
            "order_id": updated_order.id,
            "return_id": return_id,
            "refund_total": refund_total,
            "new_status": updated_order.status,
        });
        let _ = audit
            .emit(
                tenant_id,
                sec.actor.clone(),
                "order",
                Some(updated_order.id),
                "refunded",
                "order-service",
                common_audit::AuditSeverity::Info,
                None,
                changes,
                json!({"source":"order-service"}),
            )
            .await;
    }
    Ok(Json(updated_order))
}

pub async fn list_orders(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Query(params): Query<ListOrdersParams>,
) -> Result<Json<Vec<Order>>, ApiError> {
    if !sec
        .roles
        .iter()
        .any(|r| matches!(r, Role::Admin | Role::Manager | Role::Support))
    {
    return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager_or_support", trace_id: None });
    }
    let tenant_id = sec.tenant_id;

    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let offset = params.offset.unwrap_or(0).max(0);

    let sort_field = match params.sort.as_deref() {
        Some("total") => "total",
        Some("status") => "status",
        Some("payment_method") => "payment_method",
        Some("customer") | Some("customer_name") => "customer_name",
        Some("customer_email") => "customer_email",
        Some("store_id") => "store_id",
        Some("created_at") => "created_at",
        _ => "created_at",
    };

    let sort_direction = match params.direction.as_ref() {
        Some(dir) if dir.eq_ignore_ascii_case("asc") || dir.eq_ignore_ascii_case("ascending") => {
            "ASC"
        }
        _ => "DESC",
    };

    let mut builder = QueryBuilder::new(
    "SELECT id, tenant_id, total AS total, status, customer_id, customer_name, customer_email, store_id, created_at, offline, payment_method, idempotency_key FROM orders WHERE tenant_id = "
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

    if let Some(store_id) = params.store_id {
        builder.push(" AND store_id = ");
        builder.push_bind(store_id);
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

    if let Some(customer_term) = params
        .customer
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        let pattern = format!("%{}%", customer_term);
        builder.push(" AND (COALESCE(customer_name, ') ILIKE ");
        builder.push_bind(pattern.clone());
        builder.push(" OR COALESCE(customer_email, ') ILIKE ");
        builder.push_bind(pattern);
        builder.push(")");
    }

    if let Some(start_date) = params.start_date {
        if let Some(start_dt) = start_date.and_hms_opt(0, 0, 0) {
            let start_dt = Utc.from_utc_datetime(&start_dt);
            builder.push(" AND created_at >= ");
            builder.push_bind(start_dt);
        }
    }

    if let Some(end_date) = params.end_date {
        if let Some(end_dt) = end_date.and_hms_opt(23, 59, 59) {
            let end_dt = Utc.from_utc_datetime(&end_dt);
            builder.push(" AND created_at <= ");
            builder.push_bind(end_dt);
        }
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

    builder.push(" ORDER BY ");
    builder.push(sort_field);
    builder.push(" ");
    builder.push(sort_direction);
    builder.push(" LIMIT ");
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);

    let orders = builder
        .build_query_as::<Order>()
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Database error: {}", e)) })?;

    Ok(Json(orders))
}

pub async fn list_returns(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Query(params): Query<ListReturnsParams>,
) -> Result<Json<Vec<ReturnSummary>>, ApiError> {
    if !sec
        .roles
        .iter()
        .any(|r| matches!(r, Role::Admin | Role::Manager | Role::Support))
    {
    return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager_or_support", trace_id: None });
    }
    let tenant_id = sec.tenant_id;

    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let offset = params.offset.unwrap_or(0).max(0);

    let mut builder = QueryBuilder::new(
    "SELECT r.id, r.order_id, r.total AS total, r.reason, r.created_at, o.store_id \
         FROM order_returns r \
         JOIN orders o ON o.id = r.order_id \
         WHERE r.tenant_id = ",
    );
    builder.push_bind(tenant_id);

    if let Some(order_id) = params.order_id {
        builder.push(" AND r.order_id = ");
        builder.push_bind(order_id);
    }

    if let Some(store_id) = params.store_id {
        builder.push(" AND o.store_id = ");
        builder.push_bind(store_id);
    }

    if let Some(start_date) = params.start_date {
        if let Some(start_dt) = start_date.and_hms_opt(0, 0, 0) {
            let start_dt = Utc.from_utc_datetime(&start_dt);
            builder.push(" AND r.created_at >= ");
            builder.push_bind(start_dt);
        }
    }

    if let Some(end_date) = params.end_date {
        if let Some(end_dt) = end_date.and_hms_opt(23, 59, 59) {
            let end_dt = Utc.from_utc_datetime(&end_dt);
            builder.push(" AND r.created_at <= ");
            builder.push_bind(end_dt);
        }
    }

    builder.push(" ORDER BY r.created_at DESC");
    builder.push(" LIMIT ");
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);

    let raw_rows = builder
        .build()
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Database error: {}", e)) })?;

    let mut returns: Vec<ReturnSummary> = Vec::with_capacity(raw_rows.len());
    for row in raw_rows {
    let id: Uuid = row.try_get("id").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read return id: {e}")) })?;
    let order_id: Uuid = row.try_get("order_id").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read return order_id: {e}")) })?;
    let total: BigDecimal = row.try_get("total").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read return total: {e}")) })?;
        let reason: Option<String> = row.try_get("reason").ok();
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read return created_at: {e}")) })?;
        let store_id: Option<Uuid> = row.try_get("store_id").ok();
        returns.push(ReturnSummary { id, order_id, total: Money::new(total), reason, created_at, store_id });
    }

    Ok(Json(returns))
}
async fn fetch_order_detail(
    state: &AppState,
    tenant_id: Uuid,
    order_id: Uuid,
) -> Result<OrderDetail, ApiError> {
    let order = sqlx::query_as::<_, Order>(
    "SELECT id, tenant_id, total, status, customer_id, customer_name, customer_email, store_id, created_at, offline, payment_method, idempotency_key FROM orders WHERE id = $1 AND tenant_id = $2",
    )
    .bind(order_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to load order: {}", e)) })?
    .ok_or(ApiError::NotFound { code: "order_not_found", trace_id: None })?;

    let item_rows = sqlx::query(
    "SELECT product_id, product_name, quantity, returned_quantity, unit_price, line_total FROM order_items WHERE order_id = $1 ORDER BY created_at"
    )
    .bind(order_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to load order items: {}", e)) })?;

    let items = item_rows
        .into_iter()
        .map(|row| {
            let product_id: Uuid = row.try_get("product_id").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read order item product: {}", e)) })?;
            let product_name: Option<String> = row.try_get("product_name").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read order item product name: {}", e)) })?;
            let quantity: i32 = row.try_get("quantity").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read order item quantity: {}", e)) })?;
            let returned_quantity: i32 = row.try_get("returned_quantity").unwrap_or(0);
            let unit_price: BigDecimal = row.try_get("unit_price").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read order item unit price: {}", e)) })?;
            let line_total: BigDecimal = row.try_get("line_total").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read order item line total: {}", e)) })?;
            Ok(OrderLineItem {
                product_id,
                product_name,
                quantity,
                unit_price: Money::new(unit_price),
                line_total: Money::new(line_total),
                returned_quantity,
            })
        })
    .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(OrderDetail { order, items })
}

pub async fn get_order(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(order_id): Path<Uuid>,
) -> Result<Json<OrderDetail>, ApiError> {
    if !sec
        .roles
        .iter()
        .any(|r| matches!(r, Role::Admin | Role::Manager | Role::Support))
    {
    return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager_or_support", trace_id: None });
    }
    let tenant_id = sec.tenant_id;
    let detail = fetch_order_detail(&state, tenant_id, order_id).await?;
    Ok(Json(detail))
}
pub async fn get_order_receipt(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(order_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    if !sec
        .roles
        .iter()
        .any(|r| matches!(r, Role::Admin | Role::Manager | Role::Support))
    {
    return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager_or_support", trace_id: None });
    }
    let tenant_id = sec.tenant_id;
    let detail = fetch_order_detail(&state, tenant_id, order_id).await?;

    let mut body = String::new();
    body.push_str("# Receipt - Order ");
    body.push_str(&detail.order.id.to_string());
    body.push('\n');

    body.push_str("**Date:** ");
    body.push_str(&detail.order.created_at.to_rfc3339());
    body.push('\n');

    if let Some(store_id) = detail.order.store_id {
        body.push_str("**Store:** ");
        body.push_str(&store_id.to_string());
        body.push('\n');
    }

    if detail.order.customer_name.is_some() || detail.order.customer_email.is_some() {
        body.push_str("**Customer:** ");
        if let Some(name) = detail.order.customer_name.as_ref() {
            body.push_str(name);
        }
        if let Some(email) = detail.order.customer_email.as_ref() {
            if detail.order.customer_name.is_some() {
                body.push_str(" (");
                body.push_str(email);
                body.push(')');
            } else {
                body.push_str(email);
            }
        }
        body.push('\n');
    }

    body.push('\n');
    body.push_str("| Item | Qty | Price | Total |\n| --- | ---: | ---: | ---: |\n");
    for item in &detail.items {
        let name = item.product_name.as_deref().unwrap_or("Item");
        body.push_str(&format!(
            "| {} | {} | ${:.2} | ${:.2} |\n",
            name, item.quantity, item.unit_price, item.line_total
        ));
    }

    body.push('\n');
    body.push_str(&format!("**Grand Total:** ${:.2}\n", detail.order.total));
    body.push_str(&format!(
        "**Payment Method:** {}\n",
        detail.order.payment_method
    ));
    body.push_str(&format!("**Status:** {}\n", detail.order.status));
    body.push('\n');
    body.push_str("_Thank you for your business!_\n");

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/markdown; charset=utf-8"),
    );

    Ok((StatusCode::OK, headers, body).into_response())
}
pub async fn clear_offline_orders(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
) -> Result<Json<ClearOfflineResponse>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
        return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager", trace_id: None });
    }
    let tenant_id = sec.tenant_id;

    let result = sqlx::query!(
        r#"DELETE FROM orders WHERE tenant_id = $1 AND offline = TRUE"#,
        tenant_id
    )
    .execute(&state.db)
    .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to clear offline orders: {}", e)) })?;

    Ok(Json(ClearOfflineResponse {
        cleared: result.rows_affected(),
    }))
}
