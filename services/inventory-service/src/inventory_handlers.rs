use axum::{Json, http::{HeaderMap, StatusCode}};
use axum::extract::State;
use serde::Serialize;
use uuid::Uuid;
use crate::AppState;
use sqlx::query_as;

#[derive(sqlx::FromRow, Serialize)]
pub struct InventoryRecord {
    pub product_id: Uuid,
    pub tenant_id: Uuid,
    pub quantity: i32,
    pub threshold: i32,
}

pub async fn list_inventory(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<InventoryRecord>>, (StatusCode, String)> {
    // Extract tenant ID from header
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => return Err((StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID header".to_string())),
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Missing X-Tenant-ID header".to_string()));
    };

    // Query inventory records for this tenant
    let records = query_as::<_, InventoryRecord>(
        "SELECT product_id, tenant_id, quantity, threshold FROM inventory WHERE tenant_id = ",
    )
    .bind(tenant_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;

    Ok(Json(records))
}
