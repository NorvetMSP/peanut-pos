use crate::AppState;
use axum::extract::{Query, State};
use axum::{
    Json,
};
use common_security::{SecurityCtxExtractor, roles::{ensure_any_role, Role}};
use serde::{Deserialize, Serialize};
use sqlx::{query, Row};
use sqlx::query_as;
use uuid::Uuid;
use common_http_errors::ApiError;

pub(crate) const LIST_INVENTORY_SQL: &str =
    "SELECT product_id, tenant_id, quantity, threshold FROM inventory WHERE tenant_id = $1";

pub(crate) const INVENTORY_VIEW_ROLES: &[Role] = &[Role::Admin, Role::Manager, Role::Inventory];

#[derive(Debug, sqlx::FromRow, Serialize)]
pub struct InventoryRecord {
    pub product_id: Uuid,
    pub tenant_id: Uuid,
    pub quantity: i32,
    pub threshold: i32,
}

#[derive(Debug, Deserialize, Default)]
pub struct InventoryQueryParams {
    pub location_id: Option<Uuid>,
    pub location_ids: Option<String>, // CSV list of location_ids
}

pub async fn list_inventory(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Query(params): Query<InventoryQueryParams>,
) -> Result<Json<Vec<InventoryRecord>>, ApiError> {
    if ensure_any_role(&sec, INVENTORY_VIEW_ROLES).is_err() {
        return Err(ApiError::ForbiddenMissingRole { role: "manager", trace_id: sec.trace_id });
    }
    let tenant_id = sec.tenant_id;
    let records = if state.multi_location_enabled {
        if let Some(location_id) = params.location_id {
            let rows = query(
                "SELECT product_id, tenant_id, quantity, threshold FROM inventory_items WHERE tenant_id = $1 AND location_id = $2",
            )
            .bind(tenant_id)
            .bind(location_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| ApiError::internal(e, None))?;
            rows.into_iter()
                .map(|r| InventoryRecord {
                    product_id: r.get("product_id"),
                    tenant_id: r.get("tenant_id"),
                    quantity: r.get::<i32, _>("quantity"),
                    threshold: r.get::<i32, _>("threshold"),
                })
                .collect()
        } else if let Some(list) = params.location_ids.as_ref() {
            let ids: Vec<Uuid> = list
                .split(',')
                .filter_map(|s| Uuid::parse_str(s.trim()).ok())
                .collect();
            if ids.is_empty() {
                Vec::new()
            } else {
                let rows = query(
                    "SELECT product_id, tenant_id, SUM(quantity) as quantity, MIN(threshold) as threshold FROM inventory_items WHERE tenant_id = $1 AND location_id = ANY($2) GROUP BY product_id, tenant_id",
                )
                .bind(tenant_id)
                .bind(&ids)
                .fetch_all(&state.db)
                .await
                .map_err(|e| ApiError::internal(e, None))?;
                rows.into_iter()
                    .map(|r| InventoryRecord {
                        product_id: r.get("product_id"),
                        tenant_id: r.get("tenant_id"),
                        quantity: r.get::<i64, _>("quantity") as i32,
                        threshold: r.get::<i32, _>("threshold"),
                    })
                    .collect()
            }
        } else {
            let rows = query(
                "SELECT product_id, tenant_id, SUM(quantity) AS quantity, MIN(threshold) AS threshold FROM inventory_items WHERE tenant_id = $1 GROUP BY product_id, tenant_id",
            )
            .bind(tenant_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| ApiError::internal(e, None))?;
            rows.into_iter()
                .map(|r| InventoryRecord {
                    product_id: r.get("product_id"),
                    tenant_id: r.get("tenant_id"),
                    quantity: r.get::<i64, _>("quantity") as i32,
                    threshold: r.get::<i32, _>("threshold"),
                })
                .collect()
        }
    } else {
        query_as::<_, InventoryRecord>(LIST_INVENTORY_SQL)
            .bind(tenant_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| ApiError::internal(e, None))?
    };
    Ok(Json(records))
}

// Existing tests relying on AuthContext removed; new tests will be added in dedicated test module using SecurityCtxExtractor.
