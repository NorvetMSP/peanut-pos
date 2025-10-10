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
    // Back-compat: keep payment_method but prefer `payment` if provided
    pub payment_method: String,
    #[serde(default)]
    pub payment: Option<PaymentRequest>,
    pub total: BigDecimal, // accept raw, wrap later
    pub customer_id: Option<String>,
    pub customer_name: Option<String>,
    pub customer_email: Option<String>,
    pub store_id: Option<Uuid>,
    pub offline: Option<bool>,
    pub idempotency_key: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PaymentRequest {
    pub method: String, // "cash" | "card"
    pub amount_cents: i64,
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
#[allow(dead_code)]
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

// --- Refund passthrough mapping (stub) ---
#[allow(dead_code)]
async fn request_refund_via_payment_service(
    client: &Client,
    payment_base_url: &str,
    tenant_id: Uuid,
    intent_id: Option<String>,
    provider_ref: Option<String>,
    amount_minor: Option<i64>,
    currency: Option<&str>,
) -> Result<(), String> {
    // TODO: Wire to payment-service /payment_intents/refund once contracts are finalized
    let url = format!("{}/payment_intents/refund", payment_base_url.trim_end_matches('/'));
    let mut body = serde_json::json!({});
    if let Some(id) = intent_id { body["id"] = serde_json::Value::String(id); }
    if let Some(pref) = provider_ref { body["providerRef"] = serde_json::Value::String(pref); }
    if let Some(amt) = amount_minor { body["amountMinor"] = serde_json::Value::Number(serde_json::Number::from(amt)); }
    if let Some(cur) = currency { body["currency"] = serde_json::Value::String(cur.to_string()); }
    let resp = client.post(url)
        .header("Content-Type", "application/json")
        .header("X-Tenant-ID", tenant_id.to_string())
        .json(&body)
        .send().await.map_err(|e| e.to_string())?;
    if resp.status().is_success() { Ok(()) } else { Err(format!("refund request failed: {}", resp.status())) }
}

// Compute subtotal, discount, and tax for a set of items using product tax_code and DEFAULT_TAX_RATE_BPS.
#[allow(dead_code)]
async fn compute_financials_for_items(
    state: &AppState,
    tenant_id: Uuid,
    items: &[OrderItem],
    total_cents: i64,
) -> (i64, i64, i64) {
    let subtotal_cents: i64 = items.iter().map(|it| it.line_total.as_cents()).sum();
    let product_ids: Vec<Uuid> = items.iter().map(|it| it.product_id).collect();
    #[derive(sqlx::FromRow)]
    struct TaxRow { id: Uuid, tax_code: Option<String> }
    let mut taxable_subtotal_cents: i64 = 0;
    if !product_ids.is_empty() {
        if let Ok(rows) = sqlx::query_as::<_, TaxRow>(
            "SELECT id, tax_code FROM products WHERE tenant_id = $1 AND id = ANY($2)"
        )
        .bind(tenant_id)
        .bind(&product_ids)
        .fetch_all(&state.db)
        .await {
            use std::collections::HashMap as Map;
            let tax_map: Map<Uuid, Option<String>> = rows.into_iter().map(|r| (r.id, r.tax_code)).collect();
            for it in items {
                let taxable = match tax_map.get(&it.product_id).and_then(|c| c.clone()) {
                    Some(code) => is_taxable(Some(&code)),
                    None => true,
                };
                if taxable { taxable_subtotal_cents += it.line_total.as_cents(); }
            }
        } else {
            // On lookup failure, conservatively treat all as taxable
            taxable_subtotal_cents = subtotal_cents;
        }
    }
    let rate_bps = default_tax_rate_bps() as i64;
    let estimated_tax_cents = if taxable_subtotal_cents > 0 && rate_bps > 0 {
        (taxable_subtotal_cents.saturating_mul(rate_bps) + 5_000) / 10_000
    } else { 0 };
    let mut discount_cents = subtotal_cents.saturating_add(estimated_tax_cents).saturating_sub(total_cents);
    if discount_cents < 0 { discount_cents = 0; }
    (subtotal_cents, discount_cents, estimated_tax_cents)
}

fn clamp_bps2(v: i32) -> i32 { v.clamp(0, 10_000) }

async fn resolve_tax_rate_bps_with_db(
    db: &sqlx::PgPool,
    tenant_id: Uuid,
    headers: &HeaderMap,
    req_rate: Option<i32>,
    location_id: Option<Uuid>,
    pos_instance_id: Option<Uuid>,
) -> i32 {
    if let Some(bp) = req_rate { return clamp_bps2(bp); }
    if let Some(v) = headers.get("x-tax-rate-bps").and_then(|h| h.to_str().ok()).and_then(|s| s.parse::<i32>().ok()) {
        return clamp_bps2(v);
    }
    if let Some(v) = headers.get("x-tenant-tax-rate-bps").and_then(|h| h.to_str().ok()).and_then(|s| s.parse::<i32>().ok()) {
        return clamp_bps2(v);
    }
    if let Some(pos_id) = pos_instance_id {
        if let Ok(Some(row)) = sqlx::query_scalar::<_, i32>(
            "SELECT rate_bps FROM tax_rate_overrides WHERE tenant_id = $1 AND pos_instance_id = $2 ORDER BY updated_at DESC LIMIT 1"
        ).bind(tenant_id).bind(pos_id).fetch_optional(db).await { return clamp_bps2(row); }
    }
    if let Some(loc_id) = location_id {
        if let Ok(Some(row)) = sqlx::query_scalar::<_, i32>(
            "SELECT rate_bps FROM tax_rate_overrides WHERE tenant_id = $1 AND location_id = $2 AND pos_instance_id IS NULL ORDER BY updated_at DESC LIMIT 1"
        ).bind(tenant_id).bind(loc_id).fetch_optional(db).await { return clamp_bps2(row); }
    }
    if let Ok(Some(row)) = sqlx::query_scalar::<_, i32>(
        "SELECT rate_bps FROM tax_rate_overrides WHERE tenant_id = $1 AND location_id IS NULL AND pos_instance_id IS NULL ORDER BY updated_at DESC LIMIT 1"
    ).bind(tenant_id).fetch_optional(db).await { return clamp_bps2(row); }
    default_tax_rate_bps()
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

#[derive(Debug, Deserialize)]
pub struct SettlementQuery {
    pub date: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SettlementByMethod {
    pub method: String,
    pub count: i64,
    pub amount: BigDecimal,
}

#[derive(Debug, Serialize)]
pub struct SettlementReport {
    pub date: String,
    pub totals: Vec<SettlementByMethod>,
}

pub async fn get_settlement_report(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Query(q): Query<SettlementQuery>,
) -> Result<Json<SettlementReport>, ApiError> {
    if !sec
        .roles
        .iter()
        .any(|r| matches!(r, Role::Admin | Role::Manager | Role::Support))
    {
        return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager_or_support", trace_id: None });
    }

    let tenant_id = sec.tenant_id;
    let date_str = q
        .date
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let date = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
        .map_err(|_| ApiError::BadRequest { code: "invalid_date", trace_id: None, message: Some("Expected YYYY-MM-DD".into()) })?;

    #[derive(sqlx::FromRow)]
    struct SettlementRow {
        method: String,
        count: Option<i64>,
        amount: Option<BigDecimal>,
    }

    let rows = sqlx::query_as::<_, SettlementRow>(
        r#"SELECT method, COUNT(*) as count, COALESCE(SUM(amount)::NUMERIC, 0)::NUMERIC as amount
            FROM payments
            WHERE tenant_id = $1 AND status = 'captured' AND created_at::date = $2
            GROUP BY method
            ORDER BY method"#,
    )
    .bind(tenant_id)
    .bind(date)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to query settlement: {}", e)) })?;

    let mut totals = Vec::with_capacity(rows.len());
    for r in rows {
        totals.push(SettlementByMethod {
            method: r.method,
            count: r.count.unwrap_or(0),
            amount: r.amount.unwrap_or(BigDecimal::from(0)),
        });
    }

    Ok(Json(SettlementReport { date: date_str, totals }))
}

// --- Exchanges (MVP) ---
#[derive(Deserialize, Debug)]
pub struct ExchangeReturnItem { pub product_id: Uuid, pub qty: i32 }

#[derive(Deserialize, Debug)]
pub struct ExchangeNewItem { pub sku: String, pub qty: i32 }

#[derive(Deserialize, Debug)]
pub struct ExchangeRequest {
    pub return_items: Vec<ExchangeReturnItem>,
    pub new_items: Vec<ExchangeNewItem>,
    #[serde(default)] pub discount_percent_bp: Option<i32>,
    pub payment: Option<PaymentRequest>,
    pub cashier_id: Option<Uuid>,
    pub idempotency_key: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct ExchangeResponse {
    pub original_order_id: Uuid,
    pub exchange_order_id: Uuid,
    pub refund: Option<Uuid>,
    pub refunded_cents: i64,
    pub new_order_total_cents: i64,
    pub net_delta_cents: i64,
    pub net_direction: String,
}

pub async fn exchange_order(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(original_order_id): Path<Uuid>,
    headers: HeaderMap,
    auth: AuthContext,
    Json(req): Json<ExchangeRequest>,
) -> Result<Json<ExchangeResponse>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
        return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager", trace_id: None });
    }
    let tenant_id = sec.tenant_id;

    if req.return_items.is_empty() && req.new_items.is_empty() {
        return Err(ApiError::BadRequest { code: "invalid_request", trace_id: None, message: Some("Must provide return_items or new_items".into()) });
    }

    // 1) Load original order and items with returned balance
    let detail = fetch_order_detail(&state, tenant_id, original_order_id).await?;
    if !matches!(detail.order.status.as_str(), "COMPLETED" | "PAID" | "REFUNDED" | "PARTIAL_REFUNDED") {
        return Err(ApiError::BadRequest { code: "order_not_completed", trace_id: None, message: Some("Original order not in a refundable state".into()) });
    }

    // Build product_id -> refundable quantity map and order_item_id lookup
    #[derive(Clone)]
    struct ItemRow { order_item_id: Uuid, qty: i32, returned: i32, unit_price_cents: i64 }
    let mut by_product: HashMap<Uuid, ItemRow> = HashMap::new();
    {
        let rows = sqlx::query(
            "SELECT id, product_id, quantity, returned_quantity, unit_price FROM order_items WHERE order_id = $1"
        ).bind(original_order_id).fetch_all(&state.db).await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to load original order items: {}", e)) })?;
        for row in rows {
            let order_item_id: Uuid = row.try_get("id").unwrap();
            let product_id: Uuid = row.try_get("product_id").unwrap();
            let quantity: i32 = row.try_get("quantity").unwrap();
            let returned_quantity: i32 = row.try_get("returned_quantity").unwrap_or(0);
            let unit_price: BigDecimal = row.try_get("unit_price").unwrap();
            by_product.insert(product_id, ItemRow { order_item_id, qty: quantity, returned: returned_quantity, unit_price_cents: Money::new(unit_price).as_cents() });
        }
    }

    // 2) Compute refund_total_cents and update returned_quantity in a tx
    let mut tx = state.db.begin().await.map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to begin exchange tx: {}", e)) })?;
    let mut refund_total_cents: i64 = 0;
    let mut return_id: Option<Uuid> = None;
    if !req.return_items.is_empty() {
        // Validate quantities
        for it in &req.return_items {
            if it.qty <= 0 { return Err(ApiError::BadRequest { code: "invalid_quantity", trace_id: None, message: Some("Return quantities must be positive".into()) }); }
            let row = by_product.get(&it.product_id).ok_or(ApiError::BadRequest { code: "product_not_in_order", trace_id: None, message: Some("Product is not part of original order".into()) })?;
            let available = row.qty - row.returned;
            if it.qty > available { return Err(ApiError::BadRequest { code: "refundable_qty_exceeded", trace_id: None, message: Some(format!("Cannot return {} units; only {} remain", it.qty, available)) }); }
        }

        let rid = Uuid::new_v4();
        return_id = Some(rid);
        {
            let conn = tx.acquire().await.map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire conn: {}", e)) })?;
            sqlx::query("INSERT INTO order_returns (id, order_id, tenant_id, total) VALUES ($1,$2,$3,$4)")
                .bind(rid)
                .bind(original_order_id)
                .bind(tenant_id)
                .bind(Money::from_cents(0).inner())
                .execute(&mut *conn).await
                .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to insert return: {}", e)) })?;
        }
        for it in &req.return_items {
            let row = by_product.get(&it.product_id).unwrap();
            let line_cents = row.unit_price_cents.saturating_mul(it.qty as i64);
            refund_total_cents = refund_total_cents.saturating_add(line_cents);
            // update returned_quantity and insert return item row
            let conn = tx.acquire().await.map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire conn: {}", e)) })?;
            sqlx::query("UPDATE order_items SET returned_quantity = returned_quantity + $1 WHERE id = $2")
                .bind(it.qty)
                .bind(row.order_item_id)
                .execute(&mut *conn).await
                .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to update returned qty: {}", e)) })?;
            let conn2 = tx.acquire().await.map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire conn: {}", e)) })?;
            sqlx::query("INSERT INTO order_return_items (id, return_id, order_item_id, quantity, line_total) VALUES ($1,$2,$3,$4,$5)")
                .bind(Uuid::new_v4())
                .bind(return_id.unwrap())
                .bind(row.order_item_id)
                .bind(it.qty)
                .bind(Money::from_cents(line_cents).inner())
                .execute(&mut *conn2).await
                .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to insert return item: {}", e)) })?;
        }
        // finalize return total and possibly order status partial/full
        let all_returned = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM order_items WHERE order_id = $1 AND returned_quantity < quantity")
            .bind(original_order_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to compute return completion: {}", e)) })? == 0;
        let new_status = if all_returned { "REFUNDED" } else { "PARTIAL_REFUNDED" };
        let conn = tx.acquire().await.map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire conn: {}", e)) })?;
        sqlx::query("UPDATE order_returns SET total = $2 WHERE id = $1")
            .bind(return_id.unwrap())
            .bind(Money::from_cents(refund_total_cents).inner())
            .execute(&mut *conn).await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to finalize return: {}", e)) })?;
        let conn2 = tx.acquire().await.map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire conn: {}", e)) })?;
        sqlx::query("UPDATE orders SET status = $3 WHERE id = $1 AND tenant_id = $2")
            .bind(original_order_id)
            .bind(tenant_id)
            .bind(new_status)
            .execute(&mut *conn2).await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to update order status: {}", e)) })?;
    }

    // 3) Build new order from SKUs
    let new_items: Vec<NewOrderSkuItem> = req.new_items.iter().map(|i| NewOrderSkuItem { sku: i.sku.clone(), quantity: i.qty }).collect();
    let new_req = NewOrderFromSku {
        items: new_items,
        discount_percent_bp: req.discount_percent_bp,
        tax_rate_bps: None,
        location_id: None,
        pos_instance_id: None,
        payment_method: req.payment.as_ref().map(|p| p.method.clone()).unwrap_or_else(|| "cash".to_string()),
        payment: req.payment.clone(),
        customer_id: detail.order.customer_id.map(|id| id.to_string()),
        customer_name: detail.order.customer_name.clone(),
        customer_email: detail.order.customer_email.clone(),
        store_id: detail.order.store_id,
        offline: Some(detail.order.offline),
        idempotency_key: req.idempotency_key.clone(),
    };
    let created = create_order_from_skus(State(state.clone()), SecurityCtxExtractor(sec.clone()), auth, headers.clone(), Json(new_req)).await?;
    let exchange_order = created.0;

    // 4) Link the new order to the original via exchange_of_order_id
    sqlx::query("UPDATE orders SET exchange_of_order_id = $3 WHERE id = $1 AND tenant_id = $2")
        .bind(exchange_order.id)
        .bind(tenant_id)
        .bind(original_order_id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to link exchange order: {}", e)) })?;

    let new_total_cents = exchange_order.total.as_cents();
    let net_delta_cents = new_total_cents.saturating_sub(refund_total_cents);
    let net_direction = if net_delta_cents > 0 { "collect" } else if net_delta_cents < 0 { "refund" } else { "even" }.to_string();

    tx.commit().await.map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Exchange transaction commit failed: {}", e)) })?;

    Ok(Json(ExchangeResponse {
        original_order_id,
        exchange_order_id: exchange_order.id,
        refund: return_id,
        refunded_cents: refund_total_cents,
        new_order_total_cents: new_total_cents,
        net_delta_cents,
        net_direction,
    }))
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
    if std::env::var("ORDER_BYPASS_INVENTORY").ok().as_deref() == Some("1") {
        // Short-circuit inventory calls for tests/in-memory harness
        return Ok(());
    }
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
    .header("X-Roles", "Admin,Manager,Cashier")
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
    if std::env::var("ORDER_BYPASS_INVENTORY").ok().as_deref() == Some("1") {
        return Ok(());
    }
    let mut request = client
        .delete(inventory_url(
            base_url,
            &format!("/inventory/reservations/{}", order_id),
        ))
        .header("X-Tenant-ID", tenant_id.to_string())
    .header("X-Roles", "Admin,Manager,Cashier");

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
        .any(|r| matches!(r, Role::Admin | Role::Manager | Role::Support | Role::Cashier))
    {
    return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager_or_support", trace_id: None });
    }
    let tenant_id = sec.tenant_id;

    if new_order.items.is_empty() {
    return Err(ApiError::BadRequest { code: "missing_items", trace_id: None, message: Some("Order must contain at least one item".into()) });
    }

    // Determine payment method and amount (if provided)
    let mut payment_method = if let Some(p) = &new_order.payment {
        p.method.trim().to_lowercase()
    } else {
        new_order.payment_method.trim().to_lowercase()
    };
    if payment_method.is_empty() { payment_method = "cash".to_string(); }

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
    let total_cents = Money::new(new_order.total.clone()).as_cents();
    // Determine final order status based on payment semantics (mock card, cash)
    let status = match payment_method.as_str() {
        "cash" => {
            if let Some(p) = &new_order.payment {
                if p.amount_cents < total_cents { return Err(ApiError::BadRequest { code: "insufficient_cash", trace_id: None, message: Some("Cash provided is less than total".into()) }); }
                "COMPLETED"
            } else {
                // No amount provided; treat as pending until payment is added (strict MVP requires change logic)
                return Err(ApiError::BadRequest { code: "missing_amount", trace_id: None, message: Some("Cash payment requires amount_cents".into()) });
            }
        }
        "card" => {
            if let Some(p) = &new_order.payment {
                if p.amount_cents != total_cents { return Err(ApiError::BadRequest { code: "amount_mismatch", trace_id: None, message: Some("Card amount must equal order total".into()) }); }
                // Optionally create a payment intent (feature-gated)
                if state.enable_payment_intents {
                    let url = format!("{}/payment_intents", state.payment_base_url.trim_end_matches('/'));
                    let body = serde_json::json!({
                        "id": format!("pi_{}", order_id),
                        "orderId": order_id.to_string(),
                        "amountMinor": total_cents,
                        "currency": "USD",
                        "idempotencyKey": new_order.idempotency_key.clone().unwrap_or_else(|| format!("ord:{}", order_id))
                    });
                    let resp = state.http_client.post(url)
                        .header("Content-Type", "application/json")
                        .header("X-Tenant-ID", tenant_id.to_string())
                        .json(&body)
                        .send().await;
                    if let Err(err) = resp { tracing::warn!(?err, order_id = %order_id, "payment intent create failed (network)"); }
                }
                "COMPLETED"
            } else {
                return Err(ApiError::BadRequest { code: "missing_amount", trace_id: None, message: Some("Card payment requires amount_cents".into()) });
            }
        }
        _ => {
            return Err(ApiError::BadRequest { code: "invalid_method", trace_id: None, message: Some("Unsupported payment method".into()) });
        }
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

    let insert_result = {
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
    };
    let order = match insert_result {
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

    // If we have a payment, persist it within the same tx and compute change for cash
    if let Some(p) = &new_order.payment {
        let change_cents = if payment_method == "cash" { Some(p.amount_cents.saturating_sub(total_cents) as i32) } else { None };
        let conn = tx
            .acquire()
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
        sqlx::query(
            r#"INSERT INTO payments (id, tenant_id, order_id, method, amount, status, change_cents)
               VALUES ($1,$2,$3,$4,$5,$6,$7)"#
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(order.id)
        .bind(&payment_method)
        .bind(Money::from_cents(p.amount_cents).inner())
        .bind("captured")
        .bind(change_cents)
        .execute(&mut *conn)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to insert payment: {e}")) })?;
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
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
        // Build both legacy order.completed and new pos.order payloads
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
            // Emit pos.order event with computed tax/discount and SKU enrichment
            let product_ids: Vec<Uuid> = new_order.items.iter().map(|i| i.product_id).collect();
            let sku_map: std::collections::HashMap<Uuid, Option<String>> = if !product_ids.is_empty() {
                #[derive(sqlx::FromRow)]
                struct ProductSkuRow {
                    id: Uuid,
                    sku: Option<String>,
                }
                match sqlx::query_as::<_, ProductSkuRow>(
                    "SELECT id, sku FROM products WHERE tenant_id = $1 AND id = ANY($2)",
                )
                .bind(tenant_id)
                .bind(&product_ids)
                .fetch_all(&state.db)
                .await {
                    Ok(rows) => rows.into_iter().map(|r| (r.id, r.sku)).collect(),
                    Err(e) => {
                        tracing::warn!(?e, "Failed to fetch SKUs for pos.order event");
                        std::collections::HashMap::new()
                    }
                }
            } else {
                std::collections::HashMap::new()
            };
            let pos_items: Vec<serde_json::Value> = new_order.items.iter().map(|i| {
                let unit_cents = i.unit_price.as_cents();
                let line_cents = i.line_total.as_cents();
                let sku = sku_map.get(&i.product_id).and_then(|s| s.clone());
                serde_json::json!({
                    "sku": sku,
                    "name": i.product_name,
                    "qty": i.quantity,
                    "unit_price_cents": unit_cents,
                    "line_total_cents": line_cents
                })
            }).collect();
            let total_cents = Money::new(new_order.total.clone()).as_cents();
            let (_sub, discount_cents, tax_cents) = compute_financials_for_items(&state, tenant_id, &new_order.items, total_cents).await;
            let pos_evt = serde_json::json!({
                "order_id": order.id,
                "status": "paid",
                "total_cents": total_cents,
                "tax_cents": tax_cents,
                "discount_cents": discount_cents,
                "items": pos_items,
                "payment_method": order.payment_method,
                "occurred_at": chrono::Utc::now().to_rfc3339(),
            });
            if let Err(err) = state
                .kafka_producer
                .send(
                    FutureRecord::to("pos.order")
                        .payload(&pos_evt.to_string())
                        .key(&tenant_id.to_string()),
                    Duration::from_secs(0),
                )
                .await
            {
                tracing::error!("Failed to send pos.order: {:?}", err);
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
    headers: HeaderMap,
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

    // Load order snapshot (+ created_at/store_id eligible for policy)
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

    // Load order created_at and store/location id for policy checks
    let (order_created_at, order_store_id): (DateTime<Utc>, Option<Uuid>) = {
        let conn = tx
            .acquire()
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
        let row = sqlx::query(
            "SELECT created_at, store_id FROM orders WHERE id = $1 AND tenant_id = $2"
        ).bind(req.order_id).bind(tenant_id)
        .fetch_one(&mut *conn).await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read order metadata: {}", e)) })?;
        let created_at: DateTime<Utc> = row.try_get("created_at").map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to read created_at: {}", e)) })?;
        let store_id: Option<Uuid> = row.try_get("store_id").ok();
        (created_at, store_id)
    };

    // Fetch return policy (location-specific, else tenant default)
    #[derive(sqlx::FromRow)]
    struct PolicyRow { allow_window_days: i32, restock_fee_bps: i32, receipt_required: bool, manager_override_allowed: bool }
    let policy: PolicyRow = {
        let conn = tx
            .acquire()
            .await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
        // Try location first
        if let Ok(Some(row)) = sqlx::query_as::<_, PolicyRow>(
            "SELECT allow_window_days, restock_fee_bps, receipt_required, manager_override_allowed FROM return_policies WHERE tenant_id = $1 AND location_id IS NOT DISTINCT FROM $2"
        ).bind(tenant_id).bind(order_store_id).fetch_optional(&mut *conn).await {
            row
        } else if let Ok(Some(row)) = sqlx::query_as::<_, PolicyRow>(
            "SELECT allow_window_days, restock_fee_bps, receipt_required, manager_override_allowed FROM return_policies WHERE tenant_id = $1 AND location_id IS NULL ORDER BY updated_at DESC LIMIT 1"
        ).bind(tenant_id).fetch_optional(&mut *conn).await {
            row
        } else {
            PolicyRow { allow_window_days: 30, restock_fee_bps: 0, receipt_required: true, manager_override_allowed: true }
        }
    };

    // Manager override token handling (optional)
    let override_token: Option<Uuid> = headers
        .get("X-Return-Override")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok());
    let mut override_valid = false;
    if let Some(token) = override_token {
        let conn = tx
            .acquire().await
            .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to acquire transaction connection: {e}")) })?;
        if let Ok(Some(row)) = sqlx::query("SELECT used_at FROM return_overrides WHERE id = $1 AND tenant_id = $2 AND order_id = $3")
            .bind(token).bind(tenant_id).bind(req.order_id).fetch_optional(&mut *conn).await {
            let used_at: Option<DateTime<Utc>> = row.try_get("used_at").ok();
            override_valid = used_at.is_none();
            if override_valid {
                let _ = sqlx::query("UPDATE return_overrides SET used_at = NOW() WHERE id = $1").bind(token).execute(&mut *tx).await;
            }
        }
    }

    // Enforce receipt requirement and window unless override is valid and allowed
    if !(override_valid && policy.manager_override_allowed) {
        if policy.receipt_required {
            // For MVP we require that POS/backend knows original order_id; this handler always has it.
            // No-op: if order_id missing we'd reject earlier with order_not_found.
        }
        if policy.allow_window_days > 0 {
            let window = chrono::Duration::days(policy.allow_window_days as i64);
            if Utc::now() - order_created_at > window {
                return Err(ApiError::BadRequest { code: "return_window_expired", trace_id: None, message: Some(format!("Return window of {} days has expired", policy.allow_window_days)) });
            }
        }
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

    #[allow(dead_code)]
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

    // Apply restock fee
    if policy.restock_fee_bps > 0 {
        // refund_total := refund_total * (1 - bps/10000)
        let fee = BigDecimal::from(policy.restock_fee_bps);
        let discounted = &refund_total * (BigDecimal::from(10000) - fee) / BigDecimal::from(10000);
        refund_total = discounted;
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
#[derive(Deserialize)]
pub struct ReceiptQuery { pub format: Option<String> }

pub async fn get_order_receipt(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(order_id): Path<Uuid>,
    Query(q): Query<ReceiptQuery>,
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

    // Derive financials for receipt: subtotal, discount (residual), tax, total
    let subtotal_cents: i64 = detail
        .items
        .iter()
        .map(|it| it.line_total.as_cents())
        .sum();
    let product_ids: Vec<Uuid> = detail.items.iter().map(|it| it.product_id).collect();
    #[derive(sqlx::FromRow)]
    struct TaxRow { id: Uuid, tax_code: Option<String> }
    let mut taxable_subtotal_cents: i64 = 0;
    if !product_ids.is_empty() {
        let rows = sqlx::query_as::<_, TaxRow>(
            "SELECT id, tax_code FROM products WHERE tenant_id = $1 AND id = ANY($2)"
        )
        .bind(tenant_id)
        .bind(&product_ids)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
        use std::collections::HashMap as Map;
        let tax_map: Map<Uuid, Option<String>> = rows.into_iter().map(|r| (r.id, r.tax_code)).collect();
        for it in &detail.items {
            if let Some(code) = tax_map.get(&it.product_id).and_then(|c| c.clone()) {
                if is_taxable(Some(&code)) { taxable_subtotal_cents += it.line_total.as_cents(); }
            } else {
                // Missing tax_code => treat as taxable by default
                taxable_subtotal_cents += it.line_total.as_cents();
            }
        }
    }
    let rate_bps = default_tax_rate_bps() as i64;
    let estimated_tax_cents = if taxable_subtotal_cents > 0 && rate_bps > 0 {
        (taxable_subtotal_cents.saturating_mul(rate_bps) + 5_000) / 10_000
    } else { 0 };
    let total_cents = detail.order.total.as_cents();
    let mut discount_cents = subtotal_cents.saturating_add(estimated_tax_cents).saturating_sub(total_cents);
    if discount_cents < 0 { discount_cents = 0; }

    // Support format switching via query string ?format=txt for plaintext output
    // Default remains markdown
    let format = q.format.unwrap_or_else(|| "md".to_string()).to_ascii_lowercase();

    let mut body = String::new();
    if format == "txt" {
        // Plaintext per MVP example
        use std::fmt::Write as _;
        writeln!(&mut body, "NovaPOS Receipt").ok();
        writeln!(&mut body, "Order: {}  Date: {}", detail.order.id, detail.order.created_at.format("%Y-%m-%d %H:%M")).ok();
        body.push_str("-------------------------------------\n");
        writeln!(&mut body, "Qty  {:8} {:>7} {:>6}", "SKU", "Price", "Line").ok();
        for item in &detail.items {
            let name_or_sku = item.product_name.as_deref().unwrap_or("SKU");
            writeln!(&mut body, "{:<4} {:8} {:>7} {:>6}", item.quantity, name_or_sku, format!("{:.2}", item.unit_price), format!("{:.2}", item.line_total)).ok();
        }
        body.push_str("-------------------------------------\n");
        writeln!(&mut body, "Subtotal:         ${:.2}", Money::from_cents(subtotal_cents)).ok();
        writeln!(&mut body, "Discount:         ${:.2}", Money::from_cents(discount_cents)).ok();
        writeln!(&mut body, "Tax:              ${:.2}", Money::from_cents(estimated_tax_cents)).ok();
        writeln!(&mut body, "Total:            ${:.2}", detail.order.total).ok();
        // Try to fetch payment to show tendered and change (best-effort; ignore errors)
        match sqlx::query(
            r#"SELECT method, amount::FLOAT8 as amount, change_cents FROM payments WHERE order_id = $1 ORDER BY created_at DESC LIMIT 1"#
        )
        .bind(detail.order.id)
        .fetch_optional(&state.db)
        .await {
            Ok(Some(row)) => {
                let method: Option<String> = row.try_get("method").ok();
                let amount: Option<f64> = row.try_get("amount").ok();
                let change_cents: Option<i32> = row.try_get("change_cents").ok();
                if let Some(a) = amount {
                    writeln!(
                        &mut body,
                        "Paid ({}):        ${:.2}",
                        method.unwrap_or(detail.order.payment_method.clone()),
                        a
                    ).ok();
                }
                if let Some(ch) = change_cents {
                    writeln!(&mut body, "Change:           ${:.2}", Money::from_cents(ch as i64)).ok();
                }
            }
            Ok(None) | Err(_) => {
                writeln!(&mut body, "Paid ({}):        ${:.2}", detail.order.payment_method, detail.order.total).ok();
            }
        }
        body.push_str("-------------------------------------\nThank you!\n");
    } else {
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
        body.push_str(&format!("**Subtotal:** ${:.2}\n", Money::from_cents(subtotal_cents)));
        body.push_str(&format!("**Discount:** ${:.2}\n", Money::from_cents(discount_cents)));
        body.push_str(&format!("**Tax:** ${:.2}\n", Money::from_cents(estimated_tax_cents)));
        body.push_str(&format!("**Grand Total:** ${:.2}\n", detail.order.total));
        body.push_str(&format!(
            "**Payment Method:** {}\n",
            detail.order.payment_method
        ));
        body.push_str(&format!("**Status:** {}\n", detail.order.status));
        body.push('\n');
        body.push_str("_Thank you for your business!_\n");
    }

    let mut headers = HeaderMap::new();
    if format == "txt" {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    } else {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/markdown; charset=utf-8"));
    }

    Ok((StatusCode::OK, headers, body).into_response())
}

// --- Compute endpoint: accepts sku/product_id + qty, returns computed totals ---
#[derive(Deserialize, Debug, Clone)]
pub struct ComputeOrderItemInput {
    #[serde(default)]
    pub sku: Option<String>,
    #[serde(default)]
    pub product_id: Option<Uuid>,
    pub quantity: i32,
}

#[derive(Deserialize, Debug)]
pub struct ComputeOrderRequest {
    pub items: Vec<ComputeOrderItemInput>,
    /// Cart-level discount in basis points (1% = 100 bps)
    #[serde(default)]
    pub discount_percent_bp: Option<i32>,
    /// Optional context for tax resolution; not yet persisted but reserved
    #[serde(default)]
    pub location_id: Option<Uuid>,
    #[serde(default)]
    pub pos_instance_id: Option<Uuid>,
    /// Explicit override for tax rate if caller already resolved
    #[serde(default)]
    pub tax_rate_bps: Option<i32>,
}

#[derive(Serialize, Debug)]
pub struct ComputedItemSummary {
    #[serde(skip_serializing_if = "Option::is_none")] pub sku: Option<String>,
    pub product_id: Uuid,
    pub name: String,
    pub qty: i32,
    pub unit_price_cents: i64,
    pub line_subtotal_cents: i64,
    #[serde(skip_serializing_if = "Option::is_none")] pub tax_code: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct ComputeOrderResponse {
    pub items: Vec<ComputedItemSummary>,
    pub subtotal_cents: i64,
    pub discount_cents: i64,
    pub tax_cents: i64,
    pub total_cents: i64,
}

fn clamp_bps(v: i32) -> i32 { v.clamp(0, 10_000) }

fn default_tax_rate_bps() -> i32 {
    std::env::var("DEFAULT_TAX_RATE_BPS")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .map(clamp_bps)
        .unwrap_or(0)
}

fn is_taxable(tax_code: Option<&str>) -> bool {
    match tax_code.map(|s| s.to_ascii_uppercase()) {
        Some(code) if code == "EXEMPT" || code == "ZERO" || code == "NONE" => false,
        _ => true, // treat STD or missing as taxable
    }
}

// --- Admin: Tax rate overrides CRUD ---
#[derive(Serialize, sqlx::FromRow)]
pub struct TaxRateOverrideRow {
    pub tenant_id: Uuid,
    pub location_id: Option<Uuid>,
    pub pos_instance_id: Option<Uuid>,
    pub rate_bps: i32,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize, Default)]
pub struct ListTaxOverridesParams {
    pub location_id: Option<Uuid>,
    pub pos_instance_id: Option<Uuid>,
}

pub async fn list_tax_rate_overrides(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Query(params): Query<ListTaxOverridesParams>,
) -> Result<Json<Vec<TaxRateOverrideRow>>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
        return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager", trace_id: None });
    }
    let tenant_id = sec.tenant_id;

    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
        "SELECT tenant_id, location_id, pos_instance_id, rate_bps, updated_at FROM tax_rate_overrides WHERE tenant_id = "
    );
    qb.push_bind(tenant_id);
    if params.location_id.is_some() {
        qb.push(" AND location_id IS NOT DISTINCT FROM ");
        qb.push_bind(params.location_id);
    }
    if params.pos_instance_id.is_some() {
        qb.push(" AND pos_instance_id IS NOT DISTINCT FROM ");
        qb.push_bind(params.pos_instance_id);
    }
    qb.push(" ORDER BY updated_at DESC");

    let rows = qb
        .build_query_as::<TaxRateOverrideRow>()
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to list overrides: {}", e)) })?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct UpsertTaxRateOverrideRequest {
    pub location_id: Option<Uuid>,
    pub pos_instance_id: Option<Uuid>,
    pub rate_bps: i32,
}

pub async fn upsert_tax_rate_override(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Json(req): Json<UpsertTaxRateOverrideRequest>,
) -> Result<Json<TaxRateOverrideRow>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
        return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager", trace_id: None });
    }
    let tenant_id = sec.tenant_id;
    let rate = clamp_bps2(req.rate_bps);
    // Ensure table exists in dev/test; prod will have migration applied
    let _ = sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS tax_rate_overrides (
            tenant_id UUID NOT NULL,
            location_id UUID NULL,
            pos_instance_id UUID NULL,
            rate_bps INTEGER NOT NULL CHECK (rate_bps >= 0 AND rate_bps <= 10000),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"#
    ).execute(&state.db).await;

    // Delete existing row for the same scope, then insert new
    sqlx::query(
        "DELETE FROM tax_rate_overrides WHERE tenant_id = $1 AND location_id IS NOT DISTINCT FROM $2 AND pos_instance_id IS NOT DISTINCT FROM $3"
    )
    .bind(tenant_id)
    .bind(req.location_id)
    .bind(req.pos_instance_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to clear existing override: {}", e)) })?;

    let row = sqlx::query_as::<_, TaxRateOverrideRow>(
        "INSERT INTO tax_rate_overrides (tenant_id, location_id, pos_instance_id, rate_bps) VALUES ($1,$2,$3,$4) RETURNING tenant_id, location_id, pos_instance_id, rate_bps, updated_at"
    )
    .bind(tenant_id)
    .bind(req.location_id)
    .bind(req.pos_instance_id)
    .bind(rate)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to upsert override: {}", e)) })?;

    Ok(Json(row))
}

// --- Admin: Return policies CRUD (MVP stub) ---
#[derive(Serialize, sqlx::FromRow, Clone)]
pub struct ReturnPolicyRow {
    pub tenant_id: Uuid,
    pub location_id: Option<Uuid>,
    pub allow_window_days: i32,
    pub restock_fee_bps: i32,
    pub receipt_required: bool,
    pub manager_override_allowed: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize, Default)]
pub struct GetReturnPolicyParams { pub location_id: Option<Uuid> }

pub async fn get_return_policy(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Query(params): Query<GetReturnPolicyParams>,
) -> Result<Json<ReturnPolicyRow>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
        return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager", trace_id: None });
    }
    let tenant_id = sec.tenant_id;
    let _ = sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS return_policies (
            tenant_id UUID NOT NULL,
            location_id UUID NULL,
            allow_window_days INTEGER NOT NULL CHECK (allow_window_days >= 0 AND allow_window_days <= 3650),
            restock_fee_bps INTEGER NOT NULL CHECK (restock_fee_bps >= 0 AND restock_fee_bps <= 10000),
            receipt_required BOOLEAN NOT NULL DEFAULT TRUE,
            manager_override_allowed BOOLEAN NOT NULL DEFAULT TRUE,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (tenant_id, location_id)
        )"#
    ).execute(&state.db).await;

    if let Some(row) = sqlx::query_as::<_, ReturnPolicyRow>(
        "SELECT tenant_id, location_id, allow_window_days, restock_fee_bps, receipt_required, manager_override_allowed, updated_at
         FROM return_policies WHERE tenant_id = $1 AND location_id IS NOT DISTINCT FROM $2"
    ).bind(tenant_id).bind(params.location_id).fetch_optional(&state.db).await.map_err(|e| ApiError::Internal{ trace_id: None, message: Some(format!("Failed to load policy: {}", e)) })? {
        return Ok(Json(row));
    }
    if let Some(row) = sqlx::query_as::<_, ReturnPolicyRow>(
        "SELECT tenant_id, location_id, allow_window_days, restock_fee_bps, receipt_required, manager_override_allowed, updated_at
         FROM return_policies WHERE tenant_id = $1 AND location_id IS NULL ORDER BY updated_at DESC LIMIT 1"
    ).bind(tenant_id).fetch_optional(&state.db).await.map_err(|e| ApiError::Internal{ trace_id: None, message: Some(format!("Failed to load default policy: {}", e)) })? {
        return Ok(Json(row));
    }
    Ok(Json(ReturnPolicyRow {
        tenant_id,
        location_id: None,
        allow_window_days: 30,
        restock_fee_bps: 0,
        receipt_required: true,
        manager_override_allowed: true,
        updated_at: Utc::now(),
    }))
}

#[derive(Deserialize)]
pub struct UpsertReturnPolicyRequest {
    pub location_id: Option<Uuid>,
    pub allow_window_days: i32,
    pub restock_fee_bps: i32,
    pub receipt_required: bool,
    pub manager_override_allowed: bool,
}

pub async fn upsert_return_policy(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Json(req): Json<UpsertReturnPolicyRequest>,
) -> Result<Json<ReturnPolicyRow>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
        return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager", trace_id: None });
    }
    let tenant_id = sec.tenant_id;
    let _ = sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS return_policies (
            tenant_id UUID NOT NULL,
            location_id UUID NULL,
            allow_window_days INTEGER NOT NULL CHECK (allow_window_days >= 0 AND allow_window_days <= 3650),
            restock_fee_bps INTEGER NOT NULL CHECK (restock_fee_bps >= 0 AND restock_fee_bps <= 10000),
            receipt_required BOOLEAN NOT NULL DEFAULT TRUE,
            manager_override_allowed BOOLEAN NOT NULL DEFAULT TRUE,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (tenant_id, location_id)
        )"#
    ).execute(&state.db).await;
    sqlx::query("DELETE FROM return_policies WHERE tenant_id = $1 AND location_id IS NOT DISTINCT FROM $2")
        .bind(tenant_id).bind(req.location_id).execute(&state.db).await
        .map_err(|e| ApiError::Internal{ trace_id: None, message: Some(format!("Failed to clear existing policy: {}", e)) })?;
    let row = sqlx::query_as::<_, ReturnPolicyRow>(
        "INSERT INTO return_policies (tenant_id, location_id, allow_window_days, restock_fee_bps, receipt_required, manager_override_allowed)
         VALUES ($1,$2,$3,$4,$5,$6)
         RETURNING tenant_id, location_id, allow_window_days, restock_fee_bps, receipt_required, manager_override_allowed, updated_at"
    )
    .bind(tenant_id)
    .bind(req.location_id)
    .bind(req.allow_window_days.max(0).min(3650))
    .bind(req.restock_fee_bps.max(0).min(10000))
    .bind(req.receipt_required)
    .bind(req.manager_override_allowed)
    .fetch_one(&state.db).await
    .map_err(|e| ApiError::Internal{ trace_id: None, message: Some(format!("Failed to upsert policy: {}", e)) })?;
    Ok(Json(row))
}

// --- Manager override tokens (MVP stub) ---
#[derive(Serialize, sqlx::FromRow)]
pub struct ReturnOverrideRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub order_id: Uuid,
    pub reason: String,
    pub issued_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct IssueOverrideRequest { pub order_id: Uuid, pub reason: String }

pub async fn issue_return_override(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Json(req): Json<IssueOverrideRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Manager | Role::Admin)) {
        return Err(ApiError::ForbiddenMissingRole { role: "manager_or_admin", trace_id: None });
    }
    let tenant_id = sec.tenant_id;
    let _ = sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS return_overrides (
            id UUID PRIMARY KEY,
            tenant_id UUID NOT NULL,
            order_id UUID NOT NULL,
            reason TEXT NOT NULL,
            issued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            used_at TIMESTAMPTZ NULL
        )"#
    ).execute(&state.db).await;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO return_overrides (id, tenant_id, order_id, reason) VALUES ($1,$2,$3,$4)")
        .bind(id).bind(tenant_id).bind(req.order_id).bind(&req.reason)
        .execute(&state.db).await
        .map_err(|e| ApiError::Internal{ trace_id: None, message: Some(format!("Failed to issue override: {}", e)) })?;
    Ok(Json(serde_json::json!({"override_token": id})))
}

async fn compute_with_db_inner(
    db: &sqlx::PgPool,
    tenant_id: Uuid,
    headers: &HeaderMap,
    req: &ComputeOrderRequest,
) -> Result<ComputeOrderResponse, ApiError> {
    if req.items.is_empty() {
        return Err(ApiError::BadRequest { code: "missing_items", trace_id: None, message: Some("At least one item is required".into()) });
    }

    // Collect identifiers for batch lookup (and touch context fields for now)
    let _ctx_loc = req.location_id;
    let _ctx_pos = req.pos_instance_id;
    let mut want_skus: Vec<String> = Vec::new();
    let mut want_ids: Vec<Uuid> = Vec::new();
    for it in &req.items {
        if it.quantity <= 0 { return Err(ApiError::BadRequest { code: "invalid_quantity", trace_id: None, message: Some("Quantities must be positive".into()) }); }
        if let Some(s) = it.sku.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            want_skus.push(s.to_string());
        } else if let Some(id) = it.product_id {
            want_ids.push(id);
        } else {
            return Err(ApiError::BadRequest { code: "missing_identifier", trace_id: None, message: Some("Each item must include either sku or product_id".into()) });
        }
    }

    // Fetch products by SKU and by ID
    #[derive(sqlx::FromRow)]
    struct ProductRow { id: Uuid, name: String, price: BigDecimal, sku: Option<String>, tax_code: Option<String>, active: bool }

    use std::collections::HashMap as Map;
    let mut by_sku: Map<String, ProductRow> = Map::new();
    let mut by_id: Map<Uuid, ProductRow> = Map::new();

    if !want_skus.is_empty() {
        let rows = sqlx::query_as::<_, ProductRow>(
            "SELECT id, name, price, sku, tax_code, active FROM products WHERE tenant_id = $1 AND sku = ANY($2)"
        )
        .bind(tenant_id)
        .bind(&want_skus)
        .fetch_all(db)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to fetch products by SKU: {}", e)) })?;
        for r in rows { if let Some(s) = r.sku.clone() { by_sku.insert(s, r); } }
    }
    if !want_ids.is_empty() {
        let rows = sqlx::query_as::<_, ProductRow>(
            "SELECT id, name, price, sku, tax_code, active FROM products WHERE tenant_id = $1 AND id = ANY($2)"
        )
        .bind(tenant_id)
        .bind(&want_ids)
        .fetch_all(db)
        .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to fetch products by id: {}", e)) })?;
        for r in rows { by_id.insert(r.id, r); }
    }

    // Build computed item list preserving input order
    let mut items: Vec<ComputedItemSummary> = Vec::with_capacity(req.items.len());
    let mut subtotal_cents: i64 = 0;
    let mut taxable_subtotal_cents: i64 = 0;

    for it in &req.items {
        let row_opt = if let Some(s) = it.sku.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            by_sku.get(s)
        } else if let Some(id) = it.product_id { by_id.get(&id) } else { None };
        let row = match row_opt { Some(r) => r, None => {
            return Err(ApiError::NotFound { code: "product_not_found", trace_id: None });
        }};
        if !row.active {
            return Err(ApiError::BadRequest { code: "inactive_product", trace_id: None, message: Some(format!("Product {} is inactive", row.id)) });
        }
        let unit_cents = Money::new(row.price.clone()).as_cents();
        let qty = it.quantity as i64;
        let line_subtotal_cents = unit_cents.saturating_mul(qty);
        subtotal_cents = subtotal_cents.saturating_add(line_subtotal_cents);
        if is_taxable(row.tax_code.as_deref()) { taxable_subtotal_cents = taxable_subtotal_cents.saturating_add(line_subtotal_cents); }
        items.push(ComputedItemSummary {
            sku: row.sku.clone(),
            product_id: row.id,
            name: row.name.clone(),
            qty: it.quantity,
            unit_price_cents: unit_cents,
            line_subtotal_cents,
            tax_code: row.tax_code.clone(),
        });
    }

    // Compute discount
    let discount_bps = clamp_bps(req.discount_percent_bp.unwrap_or(0));
    let discount_cents = if subtotal_cents > 0 && discount_bps > 0 {
        // round half up: add half denominator before integer division
        (subtotal_cents.saturating_mul(discount_bps as i64) + 5_000) / 10_000
    } else { 0 };

    // Allocate discount proportion applicable to taxable portion
    let mut tax_cents = 0i64;
    if taxable_subtotal_cents > 0 {
        let discount_on_taxable = if subtotal_cents > 0 && discount_cents > 0 {
            (discount_cents.saturating_mul(taxable_subtotal_cents) + (subtotal_cents / 2)) / subtotal_cents
        } else { 0 };
        let taxable_net_cents = taxable_subtotal_cents.saturating_sub(discount_on_taxable).max(0);
        let rate_bps = resolve_tax_rate_bps_with_db(
            db,
            tenant_id,
            headers,
            req.tax_rate_bps,
            req.location_id,
            req.pos_instance_id,
        ).await as i64;
        tax_cents = (taxable_net_cents.saturating_mul(rate_bps) + 5_000) / 10_000;
    }

    let total_cents = subtotal_cents.saturating_sub(discount_cents).saturating_add(tax_cents);

    Ok(ComputeOrderResponse { items, subtotal_cents, discount_cents, tax_cents, total_cents })
}

pub async fn compute_order(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    headers: HeaderMap,
    Json(req): Json<ComputeOrderRequest>,
) -> Result<Json<ComputeOrderResponse>, ApiError> {
    let tenant_id = sec.tenant_id;
    if !sec
        .roles
        .iter()
        .any(|r| matches!(r, Role::Admin | Role::Manager | Role::Support | Role::Cashier))
    {
        return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager_or_support", trace_id: None });
    }
    let out = compute_with_db_inner(&state.db, tenant_id, &headers, &req).await?;
    Ok(Json(out))
}

#[cfg(all(test, feature = "integration-tests"))]
mod integration_tests {
    use super::*;
    use sqlx::Executor;

    fn cents(n: i64) -> BigDecimal { BigDecimal::from(n) / BigDecimal::from(100i64) }

    #[tokio::test]
    async fn compute_uses_tax_code_std_and_exempt() {
        // Arrange DB (skip if not available)
        let db_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postgres".to_string());
        let pool = match sqlx::PgPool::connect(&db_url).await {
            Ok(p) => p,
            Err(err) => {
                eprintln!("SKIP compute_uses_tax_code_std_and_exempt: cannot connect to TEST_DATABASE_URL: {err}");
                return;
            }
        };
        let tenant_id = Uuid::new_v4();

        let _ = pool.execute(
            r#"
            CREATE TABLE IF NOT EXISTS products (
              id uuid PRIMARY KEY,
              tenant_id uuid NOT NULL,
              name text NOT NULL,
              price numeric NOT NULL,
              sku text,
              tax_code text,
              active boolean NOT NULL DEFAULT true
            );
            "#
        ).await;
        let _ = sqlx::query("DELETE FROM products WHERE tenant_id = $1").bind(tenant_id).execute(&pool).await;

        // Insert products
        let p_std = Uuid::new_v4();
        let p_exempt = Uuid::new_v4();
        sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(p_std).bind(tenant_id).bind("Soda Can").bind(cents(199)).bind("SKU-SODA").bind(Some("STD".to_string())).bind(true)
            .execute(&pool).await.expect("insert std");
        sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(p_exempt).bind(tenant_id).bind("Bottle Water").bind(cents(149)).bind("SKU-WATER").bind(Some("EXEMPT".to_string())).bind(true)
            .execute(&pool).await.expect("insert exempt");

        // Build request and headers
        let req = ComputeOrderRequest {
            items: vec![
                ComputeOrderItemInput { sku: Some("SKU-SODA".into()), product_id: None, quantity: 2 },
                ComputeOrderItemInput { sku: Some("SKU-WATER".into()), product_id: None, quantity: 1 },
            ],
            discount_percent_bp: Some(1000),
            location_id: None,
            pos_instance_id: None,
            tax_rate_bps: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert("X-Tax-Rate-Bps", "800".parse().unwrap());

        // Act
        let resp = compute_with_db_inner(&pool, tenant_id, &headers, &req).await.expect("compute ok");

        // Assert
        assert_eq!(resp.subtotal_cents, 547);
        assert_eq!(resp.discount_cents, 55);
        assert_eq!(resp.tax_cents, 29);
        assert_eq!(resp.total_cents, 521);

        assert_eq!(resp.items.len(), 2);
        let soda = resp.items.iter().find(|i| i.sku.as_deref() == Some("SKU-SODA")).unwrap();
        assert_eq!(soda.name, "Soda Can");
        assert_eq!(soda.unit_price_cents, 199);
        assert_eq!(soda.line_subtotal_cents, 398);
    }

    #[tokio::test]
    async fn compute_uses_db_tax_override_precedence() {
        // Arrange DB (skip if not available)
        let db_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postgres".to_string());
        let pool = match sqlx::PgPool::connect(&db_url).await {
            Ok(p) => p,
            Err(err) => {
                eprintln!("SKIP compute_uses_db_tax_override_precedence: cannot connect to TEST_DATABASE_URL: {err}");
                return;
            }
        };
        let tenant_id = Uuid::new_v4();

        // Ensure tables
        let _ = pool.execute(
            r#"
            CREATE TABLE IF NOT EXISTS products (
              id uuid PRIMARY KEY,
              tenant_id uuid NOT NULL,
              name text NOT NULL,
              price numeric NOT NULL,
              sku text,
              tax_code text,
              active boolean NOT NULL DEFAULT true
            );
            "#
        ).await;
        let _ = pool.execute(
            r#"
            CREATE TABLE IF NOT EXISTS tax_rate_overrides (
              tenant_id UUID NOT NULL,
              location_id UUID NULL,
              pos_instance_id UUID NULL,
              rate_bps INTEGER NOT NULL CHECK (rate_bps >= 0 AND rate_bps <= 10000),
              updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
            "#
        ).await;

        let _ = sqlx::query("DELETE FROM products WHERE tenant_id = $1").bind(tenant_id).execute(&pool).await;
        let _ = sqlx::query("DELETE FROM tax_rate_overrides WHERE tenant_id = $1").bind(tenant_id).execute(&pool).await;

        // Seed products
        let p_std = Uuid::new_v4();
        let p_exempt = Uuid::new_v4();
        sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(p_std).bind(tenant_id).bind("Soda Can").bind(cents(199)).bind("SKU-SODA").bind(Some("STD".to_string())).bind(true)
            .execute(&pool).await.expect("insert std");
        sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(p_exempt).bind(tenant_id).bind("Bottle Water").bind(cents(149)).bind("SKU-WATER").bind(Some("EXEMPT".to_string())).bind(true)
            .execute(&pool).await.expect("insert exempt");

        // Seed tax overrides: tenant=700, location=800, pos=900
        let loc_id = Uuid::new_v4();
        let pos_id = Uuid::new_v4();
        sqlx::query("INSERT INTO tax_rate_overrides (tenant_id, location_id, pos_instance_id, rate_bps) VALUES ($1,$2,$3,$4)")
            .bind(tenant_id).bind(Option::<Uuid>::None).bind(Option::<Uuid>::None).bind(700)
            .execute(&pool).await.expect("insert tenant override");
        sqlx::query("INSERT INTO tax_rate_overrides (tenant_id, location_id, pos_instance_id, rate_bps) VALUES ($1,$2,$3,$4)")
            .bind(tenant_id).bind(Some(loc_id)).bind(Option::<Uuid>::None).bind(800)
            .execute(&pool).await.expect("insert loc override");
        sqlx::query("INSERT INTO tax_rate_overrides (tenant_id, location_id, pos_instance_id, rate_bps) VALUES ($1,$2,$3,$4)")
            .bind(tenant_id).bind(Option::<Uuid>::None).bind(Some(pos_id)).bind(900)
            .execute(&pool).await.expect("insert pos override");

        let base_items = vec![
            ComputeOrderItemInput { sku: Some("SKU-SODA".into()), product_id: None, quantity: 2 },
            ComputeOrderItemInput { sku: Some("SKU-WATER".into()), product_id: None, quantity: 1 },
        ];

        // Case 1: tenant only (expect tax 25)
        let req1 = ComputeOrderRequest { items: base_items.clone(), discount_percent_bp: Some(1000), location_id: None, pos_instance_id: None, tax_rate_bps: None };
        let headers1 = HeaderMap::new();
        let resp1 = compute_with_db_inner(&pool, tenant_id, &headers1, &req1).await.expect("compute ok");
        assert_eq!(resp1.subtotal_cents, 547);
        assert_eq!(resp1.discount_cents, 55);
        assert_eq!(resp1.tax_cents, 25);
        assert_eq!(resp1.total_cents, 517);

        // Case 2: location (expect tax 29)
        let req2 = ComputeOrderRequest { items: base_items.clone(), discount_percent_bp: Some(1000), location_id: Some(loc_id), pos_instance_id: None, tax_rate_bps: None };
        let resp2 = compute_with_db_inner(&pool, tenant_id, &HeaderMap::new(), &req2).await.expect("compute ok");
        assert_eq!(resp2.tax_cents, 29);

        // Case 3: pos takes precedence (expect tax 32)
        let req3 = ComputeOrderRequest { items: base_items.clone(), discount_percent_bp: Some(1000), location_id: Some(loc_id), pos_instance_id: Some(pos_id), tax_rate_bps: None };
        let resp3 = compute_with_db_inner(&pool, tenant_id, &HeaderMap::new(), &req3).await.expect("compute ok");
        assert_eq!(resp3.tax_cents, 32);
    }
}
pub async fn clear_offline_orders(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
) -> Result<Json<ClearOfflineResponse>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
        return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager", trace_id: None });
    }
    let tenant_id = sec.tenant_id;

    let result = sqlx::query(
        r#"DELETE FROM orders WHERE tenant_id = $1 AND offline = TRUE"#
    )
    .bind(tenant_id)
    .execute(&state.db)
    .await
        .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to clear offline orders: {}", e)) })?;

    Ok(Json(ClearOfflineResponse {
        cleared: result.rows_affected(),
    }))
}

// --- Create order from SKUs: resolve items and compute totals server-side ---
#[derive(Deserialize, Debug)]
pub struct NewOrderSkuItem { pub sku: String, pub quantity: i32 }

#[derive(Deserialize, Debug)]
pub struct NewOrderFromSku {
    pub items: Vec<NewOrderSkuItem>,
    #[serde(default)] pub discount_percent_bp: Option<i32>,
    #[serde(default)] pub tax_rate_bps: Option<i32>,
    #[serde(default)] pub location_id: Option<Uuid>,
    #[serde(default)] pub pos_instance_id: Option<Uuid>,
    pub payment_method: String,
    #[serde(default)] pub payment: Option<PaymentRequest>,
    pub customer_id: Option<String>,
    pub customer_name: Option<String>,
    pub customer_email: Option<String>,
    pub store_id: Option<Uuid>,
    pub offline: Option<bool>,
    pub idempotency_key: Option<String>,
}

pub async fn create_order_from_skus(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    auth: AuthContext,
    headers: HeaderMap,
    Json(req): Json<NewOrderFromSku>,
) -> Result<Json<Order>, ApiError> {
    if !sec
        .roles
        .iter()
        .any(|r| matches!(r, Role::Admin | Role::Manager | Role::Support | Role::Cashier))
    {
        return Err(ApiError::ForbiddenMissingRole { role: "admin_or_manager_or_support", trace_id: None });
    }
    if req.items.is_empty() { return Err(ApiError::BadRequest { code: "missing_items", trace_id: None, message: Some("Order must contain at least one item".into()) }); }

    let tenant_id = sec.tenant_id;
    let mut want_skus: Vec<String> = Vec::with_capacity(req.items.len());
    for it in &req.items {
        if it.quantity <= 0 { return Err(ApiError::BadRequest { code: "invalid_quantity", trace_id: None, message: Some("Quantities must be positive".into()) }); }
        let s = it.sku.trim(); if s.is_empty() { return Err(ApiError::BadRequest { code: "missing_sku", trace_id: None, message: Some("SKU cannot be blank".into()) }); }
        want_skus.push(s.to_string());
    }

    #[derive(sqlx::FromRow)]
    struct ProductRow { id: Uuid, name: String, price: BigDecimal, sku: Option<String>, tax_code: Option<String>, active: bool }
    let rows = sqlx::query_as::<_, ProductRow>(
        "SELECT id, name, price, sku, tax_code, active FROM products WHERE tenant_id = $1 AND sku = ANY($2)"
    )
    .bind(tenant_id)
    .bind(&want_skus)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("Failed to fetch products by SKU: {}", e)) })?;
    use std::collections::HashMap as Map;
    let mut by_sku: Map<String, ProductRow> = Map::new();
    for r in rows { if let Some(s) = r.sku.clone() { by_sku.insert(s, r); } }

    // Build order items and compute totals
    let mut order_items: Vec<OrderItem> = Vec::with_capacity(req.items.len());
    let mut subtotal_cents: i64 = 0;
    let mut taxable_subtotal_cents: i64 = 0;
    for it in &req.items {
        let s = it.sku.trim();
        let r = by_sku.get(s).ok_or(ApiError::NotFound { code: "product_not_found", trace_id: None })?;
        if !r.active { return Err(ApiError::BadRequest { code: "inactive_product", trace_id: None, message: Some(format!("Product {} is inactive", r.id)) }); }
        let unit_cents = Money::new(r.price.clone()).as_cents();
        let qty = it.quantity as i64;
        let line_subtotal = unit_cents.saturating_mul(qty);
        subtotal_cents = subtotal_cents.saturating_add(line_subtotal);
        if is_taxable(r.tax_code.as_deref()) { taxable_subtotal_cents = taxable_subtotal_cents.saturating_add(line_subtotal); }
        order_items.push(OrderItem {
            product_id: r.id,
            product_name: Some(r.name.clone()),
            quantity: it.quantity,
            unit_price: Money::from_cents(unit_cents),
            line_total: Money::from_cents(line_subtotal),
        });
    }

    let discount_bps = req.discount_percent_bp.unwrap_or(0).clamp(0, 10_000);
    let discount_cents = if subtotal_cents > 0 && discount_bps > 0 {
        (subtotal_cents.saturating_mul(discount_bps as i64) + 5_000) / 10_000
    } else { 0 };
    let discount_on_taxable = if subtotal_cents > 0 && discount_cents > 0 {
        (discount_cents.saturating_mul(taxable_subtotal_cents) + (subtotal_cents / 2)) / subtotal_cents
    } else { 0 };
    let taxable_net = taxable_subtotal_cents.saturating_sub(discount_on_taxable).max(0);
    let tax_rate_bps = resolve_tax_rate_bps_with_db(
        &state.db,
        tenant_id,
        &headers,
        req.tax_rate_bps,
        req.location_id,
        req.pos_instance_id,
    ).await as i64;
    let tax_cents = (taxable_net.saturating_mul(tax_rate_bps) + 5_000) / 10_000;
    let total_cents = subtotal_cents.saturating_sub(discount_cents).saturating_add(tax_cents);

    // Construct a NewOrder and delegate to existing create_order by reusing its persistence path
    let new_order = NewOrder {
        items: order_items,
        payment_method: req.payment_method,
        payment: req.payment,
        total: Money::from_cents(total_cents).into(),
        customer_id: req.customer_id,
        customer_name: req.customer_name,
        customer_email: req.customer_email,
        store_id: req.store_id,
        offline: req.offline,
        idempotency_key: req.idempotency_key,
    };

    // Call inner create_order logic directly instead of HTTP roundtrip
    create_order(State(state), SecurityCtxExtractor(sec), auth, Json(new_order)).await
}
