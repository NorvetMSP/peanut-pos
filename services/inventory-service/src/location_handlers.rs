use crate::AppState;
use axum::{extract::State, Json};
use common_security::{SecurityCtxExtractor, roles::{ensure_any_role, Role}};
use common_http_errors::ApiError;
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

const LOCATION_ROLES: &[Role] = &[Role::Admin, Role::Manager, Role::Inventory]; // restricted roles

#[derive(Debug, Serialize)]
pub struct LocationRecord {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub active: bool,
}

pub async fn list_locations(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
) -> Result<Json<Vec<LocationRecord>>, ApiError> {
    if ensure_any_role(&sec, LOCATION_ROLES).is_err() {
        return Err(ApiError::ForbiddenMissingRole { role: "manager", trace_id: sec.trace_id });
    }
    let tenant_id = sec.tenant_id;
    if !state.multi_location_enabled {
        return Ok(Json(vec![]));
    }
    let rows = sqlx::query("SELECT id, code, name, active FROM locations WHERE tenant_id = $1 ORDER BY code")
        .bind(tenant_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::internal(e, None))?;

    Ok(Json(rows.into_iter().map(|r| LocationRecord {
        id: r.get("id"),
        code: r.get::<String, _>("code"),
        name: r.get::<String, _>("name"),
        active: r.get::<bool, _>("active"),
    }).collect()))
}
