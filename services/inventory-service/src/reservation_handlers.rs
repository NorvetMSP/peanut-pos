use crate::{AppState, DEFAULT_THRESHOLD}; // DEFAULT_THRESHOLD now defined in lib
use axum::extract::{Path, State};
use axum::{ Json };
use common_security::{SecurityCtxExtractor, Capability, ensure_capability};
use common_http_errors::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::{query, query_as, query_scalar, Row}; // dynamic + typed queries
use std::collections::HashMap;
use uuid::Uuid;

// Legacy RESERVATION_ROLES removed; capability Payment/Inventory reservations mapped to InventoryView + (future) InventoryWrite if introduced.

#[derive(Debug, Deserialize)]
pub struct ReservationItemPayload {
    pub product_id: Uuid,
    pub quantity: i32,
    pub location_id: Option<Uuid>, // optional until multi-location feature enabled
}

#[derive(Debug, Deserialize)]
pub struct CreateReservationRequest {
    pub order_id: Uuid,
    pub items: Vec<ReservationItemPayload>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReservationItem {
    pub product_id: Uuid,
    pub quantity: i32,
    pub location_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct ReservationResponse {
    pub order_id: Uuid,
    pub items: Vec<ReservationItem>,
}

#[derive(Debug, Serialize)]
pub struct ReleaseResponse {
    pub order_id: Uuid,
    pub released: Vec<ReservationItem>,
}

pub async fn create_reservation(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Json(payload): Json<CreateReservationRequest>,
) -> Result<Json<ReservationResponse>, ApiError> {
    ensure_capability(&sec, Capability::InventoryView)
        .map_err(|_| ApiError::ForbiddenMissingRole { role: "inventory_view", trace_id: sec.trace_id })?;
    let tenant_id = sec.tenant_id;

    if payload.items.is_empty() {
        return Err(ApiError::BadRequest { code: "empty_reservation", trace_id: None, message: Some("Reservation must include at least one item".into()) });
    }

    let mut condensed: HashMap<Uuid, i32> = HashMap::new();
    for item in payload.items.iter() {
        if item.quantity <= 0 {
            return Err(ApiError::BadRequest { code: "invalid_quantity", trace_id: None, message: Some(format!("Quantity for product {} must be positive", item.product_id)) });
        }
        *condensed.entry(item.product_id).or_insert(0) += item.quantity;
    }

    let mut tx = state
        .db
        .begin()
    .await
    .map_err(|err| ApiError::internal(err, None))?;

    let existing = query_scalar::<_, i64>(
        "SELECT 1 FROM inventory_reservations WHERE order_id = $1",
    )
    .bind(payload.order_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| ApiError::internal(err, None))?;

    if existing.is_some() {
        return Err(ApiError::BadRequest { code: "reservation_exists", trace_id: None, message: Some("Reservation already exists for this order".into()) });
    }

    let mut reserved_items = Vec::with_capacity(condensed.len());

    for (product_id, quantity) in condensed.iter() {
        // Determine a candidate location for this product (first occurrence) if provided.
        let loc = payload
            .items
            .iter()
            .find(|i| i.product_id == *product_id)
            .and_then(|i| i.location_id);

        if state.multi_location_enabled {
            // Multi-location: compute available = sum(inventory_items at location) - active reservations at that location.
            if let Some(location_id) = loc {
                let inv_row = query(
                    "SELECT quantity FROM inventory_items WHERE tenant_id = $1 AND product_id = $2 AND location_id = $3 FOR UPDATE",
                )
                .bind(tenant_id)
                .bind(product_id)
                .bind(location_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| ApiError::internal(e, None))?;
                let current_quantity: i32 = inv_row.map(|r| r.get::<i32, _>("quantity")).unwrap_or(0);
                let reserved_total: i64 = query(
                    "SELECT COALESCE(SUM(quantity),0) AS total FROM inventory_reservations WHERE tenant_id = $1 AND product_id = $2 AND location_id = $3 AND status = 'ACTIVE'",
                )
                .bind(tenant_id)
                .bind(product_id)
                .bind(location_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| ApiError::internal(e, None))?
                .get::<i64, _>("total");
                let available = current_quantity - reserved_total as i32;
                if *quantity > available {
                    return Err(ApiError::BadRequest { code: "insufficient_stock", trace_id: None, message: Some(format!("Insufficient stock for product {} at location {} (requested {}, available {})", product_id, location_id, quantity, available)) });
                }
            }

            let ttl_secs = state.reservation_default_ttl.as_secs() as i64;
            let mut ins = query("INSERT INTO inventory_reservations (order_id, tenant_id, product_id, quantity, location_id, expires_at) VALUES ($1,$2,$3,$4,$5, NOW() + ($6 * INTERVAL '1 second'))");
            ins = ins
                .bind(payload.order_id)
                .bind(tenant_id)
                .bind(product_id)
                .bind(quantity)
                .bind(loc)
                .bind(ttl_secs);
            ins.execute(&mut *tx)
                .await
                .map_err(|e| ApiError::internal(e, None))?;
            reserved_items.push(ReservationItem {
                product_id: *product_id,
                quantity: *quantity,
                location_id: loc,
            });
        } else {
            // Legacy single-inventory path
            let inventory_row = query(
                "SELECT quantity FROM inventory WHERE tenant_id = $1 AND product_id = $2 FOR UPDATE",
            )
            .bind(tenant_id)
            .bind(product_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|err| ApiError::internal(err, None))?;
            let current_quantity = inventory_row
                .map(|row| row.get::<i32, _>("quantity"))
                .unwrap_or(0);
            let reserved_total: i64 = query_scalar::<_, i64>(
                "SELECT COALESCE(SUM(quantity), 0) FROM inventory_reservations WHERE tenant_id = $1 AND product_id = $2",
            )
            .bind(tenant_id)
            .bind(product_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|err| ApiError::internal(err, None))?;
            let available = current_quantity - reserved_total as i32;
            if *quantity > available {
                return Err(ApiError::BadRequest { code: "insufficient_stock", trace_id: None, message: Some(format!("Insufficient stock for product {} (requested {}, available {})", product_id, quantity, available)) });
            }
            query(
                "INSERT INTO inventory_reservations (order_id, tenant_id, product_id, quantity) VALUES ($1, $2, $3, $4)",
            )
            .bind(payload.order_id)
            .bind(tenant_id)
            .bind(product_id)
            .bind(quantity)
            .execute(&mut *tx)
            .await
            .map_err(|err| ApiError::internal(err, None))?;
            reserved_items.push(ReservationItem {
                product_id: *product_id,
                quantity: *quantity,
                location_id: None,
            });
        }
    }

    tx.commit().await.map_err(|err| ApiError::internal(err, None))?;

    // Emit audit event (best-effort)
    let _event = serde_json::json!({
        "action": "inventory.reservation.created",
        "schema_version": 1,
        "tenant_id": tenant_id,
        "order_id": payload.order_id,
        "items": reserved_items.iter().map(|i| serde_json::json!({
            "product_id": i.product_id,
            "quantity": i.quantity,
            "location_id": i.location_id,
        })).collect::<Vec<_>>(),
    });
    #[cfg(feature = "kafka")]
    if let Err(_err) = state.kafka_producer.send(
        rdkafka::producer::FutureRecord::to("audit.events")
            .payload(&_event.to_string())
            .key(&tenant_id.to_string()),
        std::time::Duration::from_secs(0),
    ).await {
        state.metrics.audit_emit_failures.inc();
    }

    Ok(Json(ReservationResponse {
        order_id: payload.order_id,
        items: reserved_items,
    }))
}

pub async fn release_reservation(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(order_id): Path<Uuid>,
) -> Result<Json<ReleaseResponse>, ApiError> {
    ensure_capability(&sec, Capability::InventoryView)
        .map_err(|_| ApiError::ForbiddenMissingRole { role: "inventory_view", trace_id: sec.trace_id })?;
    let tenant_id = sec.tenant_id;

    let mut tx = state
        .db
        .begin()
    .await
    .map_err(|err| ApiError::internal(err, None))?;

    let rows = if state.multi_location_enabled {
        let raw = query(
            "DELETE FROM inventory_reservations WHERE order_id = $1 AND tenant_id = $2 RETURNING product_id, quantity, location_id",
        )
        .bind(order_id)
        .bind(tenant_id)
        .fetch_all(&mut *tx)
    .await
    .map_err(|err| ApiError::internal(err, None))?;
        raw.into_iter()
            .map(|r| ReservationItem {
                product_id: r.get("product_id"),
                quantity: r.get("quantity"),
                location_id: r.get("location_id"),
            })
            .collect::<Vec<_>>()
    } else {
        let legacy = query_as::<_, LegacyReservationRow>(
            "DELETE FROM inventory_reservations WHERE order_id = $1 AND tenant_id = $2 RETURNING product_id, quantity",
        )
        .bind(order_id)
        .bind(tenant_id)
        .fetch_all(&mut *tx)
    .await
    .map_err(|err| ApiError::internal(err, None))?;
        legacy.into_iter().map(|r| ReservationItem { product_id: r.product_id, quantity: r.quantity, location_id: None }).collect()
    };

    for item in rows.iter() {
        if item.quantity <= 0 {
            continue;
        }

        let mut attempts = 0;
        loop {
            match query(
                "UPDATE inventory SET quantity = quantity + $1 WHERE product_id = $2 AND tenant_id = $3 RETURNING quantity"
            )
            .bind(item.quantity)
            .bind(item.product_id)
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await
            {
                Ok(Some(_)) => break,
                Ok(None) if attempts == 0 => {
                    attempts += 1;
                    query(
                        "INSERT INTO inventory (product_id, tenant_id, quantity, threshold) VALUES ($1, $2, $3, $4) ON CONFLICT(product_id, tenant_id) DO NOTHING"
                    )
                    .bind(item.product_id)
                    .bind(tenant_id)
                    .bind(item.quantity)
                    .bind(DEFAULT_THRESHOLD)
                    .execute(&mut *tx)
                    .await
                    .map_err(|err| ApiError::internal(err, None))?;
                    continue;
                }
                Ok(None) => {
                    tracing::warn!(
                        order_id = %order_id,
                        tenant_id = %tenant_id,
                        product_id = %item.product_id,
                        "Inventory row missing during reservation release"
                    );
                    break;
                }
                Err(err) => {
                    return Err(ApiError::internal(err, None));
                }
            }
        }
    }

    tx.commit().await.map_err(|err| ApiError::internal(err, None))?;

    // (No audit emit when kafka disabled)
    Ok(Json(ReleaseResponse { order_id, released: rows }))
}

#[derive(sqlx::FromRow)]
struct LegacyReservationRow {
    product_id: Uuid,
    quantity: i32,
}
