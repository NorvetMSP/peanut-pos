use crate::AppState;
use axum::extract::State;
use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Serialize;
use sqlx::query_as;
use uuid::Uuid;

const LIST_INVENTORY_SQL: &str =
    "SELECT product_id, tenant_id, quantity, threshold FROM inventory WHERE tenant_id = $1";

#[derive(Debug, sqlx::FromRow, Serialize)]
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
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Invalid X-Tenant-ID header".to_string(),
                ))
            }
        }
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing X-Tenant-ID header".to_string(),
        ));
    };

    // Query inventory records for this tenant
    let records = query_as::<_, InventoryRecord>(LIST_INVENTORY_SQL)
        .bind(tenant_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            )
        })?;

    Ok(Json(records))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use sqlx::postgres::PgPoolOptions;

    #[test]
    fn list_inventory_query_uses_parameter_placeholder() {
        assert_eq!(
            LIST_INVENTORY_SQL,
            "SELECT product_id, tenant_id, quantity, threshold FROM inventory WHERE tenant_id = $1"
        );
    }

    #[tokio::test]
    async fn list_inventory_requires_header() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@localhost:5432/inventory_tests")
            .expect("should build lazy postgres pool");
        let result = list_inventory(State(AppState { db: pool }), HeaderMap::new()).await;
        let (status, _) = result.expect_err("missing header should fail");
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
