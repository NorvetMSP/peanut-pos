use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use uuid::Uuid;

use crate::{AppState, Stats};

pub async fn get_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Stats>, (StatusCode, String)> {
    let tenant_id = extract_tenant_id(&headers)?;

    let map = state.data.lock().unwrap();
    let stats = map.get(&tenant_id).copied().unwrap_or_default();

    Ok(Json(stats))
}

fn extract_tenant_id(headers: &HeaderMap) -> Result<Uuid, (StatusCode, String)> {
    match headers
        .get("X-Tenant-ID")
        .and_then(|hdr| hdr.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
    {
        Some(id) => Ok(id),
        None => Err((
            StatusCode::BAD_REQUEST,
            "Invalid or missing X-Tenant-ID header".to_string(),
        )),
    }
}
