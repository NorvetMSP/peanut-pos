use crate::{AppState, DEFAULT_THRESHOLD};
use axum::extract::{Path, State};
use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};
use common_auth::{
    ensure_role, tenant_id_from_request, AuthContext, ROLE_ADMIN, ROLE_CASHIER, ROLE_MANAGER,
    ROLE_SUPER_ADMIN,
};
use serde::{Deserialize, Serialize};
use sqlx::query_as; // Ensure sqlx::query_as macro is imported
use std::collections::HashMap;
use uuid::Uuid;

const RESERVATION_ROLES: &[&str] = &[ROLE_SUPER_ADMIN, ROLE_ADMIN, ROLE_MANAGER, ROLE_CASHIER];

#[derive(Debug, Deserialize)]
pub struct ReservationItemPayload {
    pub product_id: Uuid,
    pub quantity: i32,
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
    auth: AuthContext,
    headers: HeaderMap,
    Json(payload): Json<CreateReservationRequest>,
) -> Result<Json<ReservationResponse>, (StatusCode, String)> {
    ensure_role(&auth, RESERVATION_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    if payload.items.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Reservation must include at least one item".to_string(),
        ));
    }

    let mut condensed: HashMap<Uuid, i32> = HashMap::new();
    for item in payload.items.iter() {
        if item.quantity <= 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Quantity for product {} must be positive", item.product_id),
            ));
        }
        *condensed.entry(item.product_id).or_insert(0) += item.quantity;
    }

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let existing = sqlx::query_scalar!(
        "SELECT 1 FROM inventory_reservations WHERE order_id = $1",
        payload.order_id
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    if existing.is_some() {
        return Err((
            StatusCode::CONFLICT,
            "Reservation already exists for this order".to_string(),
        ));
    }

    let mut reserved_items = Vec::with_capacity(condensed.len());

    for (product_id, quantity) in condensed.iter() {
        let inventory_row = sqlx::query!(
            "SELECT quantity FROM inventory WHERE tenant_id = $1 AND product_id = $2 FOR UPDATE",
            tenant_id,
            product_id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

        let current_quantity = inventory_row.map(|row| row.quantity).unwrap_or(0);

        let reserved_total: i64 = {
            let sum_opt = sqlx::query_scalar!(
                "SELECT COALESCE(SUM(quantity), 0) FROM inventory_reservations WHERE tenant_id = $1 AND product_id = $2",
                tenant_id,
                product_id
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

            sum_opt.flatten().unwrap_or(0)
        };

        let available = current_quantity - reserved_total as i32;
        if *quantity > available {
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "Insufficient stock for product {} (requested {}, available {})",
                    product_id, quantity, available
                ),
            ));
        }

        sqlx::query!(
            "INSERT INTO inventory_reservations (order_id, tenant_id, product_id, quantity) VALUES ($1, $2, $3, $4)",
            payload.order_id,
            tenant_id,
            product_id,
            quantity
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

        reserved_items.push(ReservationItem {
            product_id: *product_id,
            quantity: *quantity,
        });
    }

    tx.commit()
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    Ok(Json(ReservationResponse {
        order_id: payload.order_id,
        items: reserved_items,
    }))
}

pub async fn release_reservation(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
) -> Result<Json<ReleaseResponse>, (StatusCode, String)> {
    ensure_role(&auth, RESERVATION_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let rows = query_as!(
        ReservationItem,
        "DELETE FROM inventory_reservations WHERE order_id = $1 AND tenant_id = $2 RETURNING product_id, quantity",
        order_id,
        tenant_id
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    for item in rows.iter() {
        if item.quantity <= 0 {
            continue;
        }

        let mut attempts = 0;
        loop {
            match sqlx::query!(
                "UPDATE inventory SET quantity = quantity + $1 WHERE product_id = $2 AND tenant_id = $3 RETURNING quantity",
                item.quantity,
                item.product_id,
                tenant_id
            )
            .fetch_optional(&mut *tx)
            .await
            {
                Ok(Some(_)) => break,
                Ok(None) if attempts == 0 => {
                    attempts += 1;
                    sqlx::query!(
                        "INSERT INTO inventory (product_id, tenant_id, quantity, threshold) VALUES ($1, $2, $3, $4) ON CONFLICT (product_id, tenant_id) DO NOTHING",
                        item.product_id,
                        tenant_id,
                        item.quantity,
                        DEFAULT_THRESHOLD
                    )
                    .execute(&mut *tx)
                    .await
                    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
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
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        err.to_string(),
                    ));
                }
            }
        }
    }

    tx.commit()
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    Ok(Json(ReleaseResponse {
        order_id,
        released: rows,
    }))
}
