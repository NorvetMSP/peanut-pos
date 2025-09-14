use axum::{Json, http::{HeaderMap, StatusCode}};
use uuid::Uuid;
use crate::{AppState, Stats};

pub async fn get_summary(
    State(state): State<AppState>,
    headers: HeaderMap
) -> Result<Json<Stats>, (StatusCode, String)> {
    // Extract tenant ID from header
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => return Err((StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID header".to_string()))
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Missing X-Tenant-ID header".to_string()));
    };
    let map = state.data.lock().unwrap();
    let stats = map.get(&tenant_id).copied().unwrap_or_default();
    Ok(Json(stats))
}
