use crate::AppState;
use axum::{extract::State, http::{HeaderMap, StatusCode}, Json};
use common_auth::{ensure_role, tenant_id_from_request, AuthContext, ROLE_ADMIN, ROLE_MANAGER, ROLE_SUPER_ADMIN};
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

const LOCATION_ROLES: &[&str] = &[ROLE_SUPER_ADMIN, ROLE_ADMIN, ROLE_MANAGER]; // cashier removed for tighter access

#[derive(Debug, Serialize)]
pub struct LocationRecord {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub active: bool,
}

pub async fn list_locations(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
) -> Result<Json<Vec<LocationRecord>>, (StatusCode, String)> {
    ensure_role(&auth, LOCATION_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;
    if !state.multi_location_enabled {
        return Ok(Json(vec![]));
    }
    let rows = sqlx::query("SELECT id, code, name, active FROM locations WHERE tenant_id = $1 ORDER BY code")
        .bind(tenant_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rows.into_iter().map(|r| LocationRecord {
        id: r.get("id"),
        code: r.get::<String, _>("code"),
        name: r.get::<String, _>("name"),
        active: r.get::<bool, _>("active"),
    }).collect()))
}
