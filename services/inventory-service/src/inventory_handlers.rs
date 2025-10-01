use crate::AppState;
use axum::extract::{Query, State};
use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};
use common_auth::{
    ensure_role, tenant_id_from_request, AuthContext, ROLE_ADMIN, ROLE_CASHIER, ROLE_MANAGER,
    ROLE_SUPER_ADMIN,
};
use serde::{Deserialize, Serialize};
use sqlx::{query, Row};
use sqlx::query_as;
use uuid::Uuid;

pub(crate) const LIST_INVENTORY_SQL: &str =
    "SELECT product_id, tenant_id, quantity, threshold FROM inventory WHERE tenant_id = $1";

// Multi-location variants (aggregation + per-location)
pub(crate) const LIST_INVENTORY_ITEMS_AGG_SQL: &str =
    "SELECT product_id, tenant_id, SUM(quantity) AS quantity, MIN(threshold) AS threshold FROM inventory_items WHERE tenant_id = $1 GROUP BY product_id, tenant_id";
pub(crate) const LIST_INVENTORY_ITEMS_BY_LOC_SQL: &str =
    "SELECT product_id, tenant_id, quantity, threshold FROM inventory_items WHERE tenant_id = $1 AND location_id = $2";

pub(crate) const INVENTORY_VIEW_ROLES: &[&str] =
    &[ROLE_SUPER_ADMIN, ROLE_ADMIN, ROLE_MANAGER, ROLE_CASHIER];

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
}

pub async fn list_inventory(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Query(params): Query<InventoryQueryParams>,
) -> Result<Json<Vec<InventoryRecord>>, (StatusCode, String)> {
    ensure_role(&auth, INVENTORY_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;
    let records = if state.multi_location_enabled {
        if let Some(location_id) = params.location_id {
            let rows = query(
                "SELECT product_id, tenant_id, quantity, threshold FROM inventory_items WHERE tenant_id = $1 AND location_id = $2",
            )
            .bind(tenant_id)
            .bind(location_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;
            rows.into_iter()
                .map(|r| InventoryRecord {
                    product_id: r.get("product_id"),
                    tenant_id: r.get("tenant_id"),
                    quantity: r.get::<i32, _>("quantity"),
                    threshold: r.get::<i32, _>("threshold"),
                })
                .collect()
        } else {
            let rows = query(
                "SELECT product_id, tenant_id, SUM(quantity) AS quantity, MIN(threshold) AS threshold FROM inventory_items WHERE tenant_id = $1 GROUP BY product_id, tenant_id",
            )
            .bind(tenant_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;
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
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?
    };
    Ok(Json(records))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use chrono::Utc;
    use common_auth::{Claims, JwtConfig, JwtVerifier};
    use sqlx::postgres::PgPoolOptions;
    use std::sync::Arc;

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
        let verifier = Arc::new(JwtVerifier::new(JwtConfig::new("issuer", "audience")));
        let state = AppState {
            db: pool,
            jwt_verifier: verifier,
            multi_location_enabled: false,
            reservation_default_ttl: std::time::Duration::from_secs(900),
            reservation_expiry_sweep: std::time::Duration::from_secs(60),
        };

        let tenant_id = Uuid::new_v4();
        let claims = Claims {
            subject: Uuid::new_v4(),
            tenant_id,
            roles: vec!["admin".to_string()],
            expires_at: Utc::now() + chrono::Duration::hours(1),
            issued_at: Some(Utc::now()),
            issuer: "issuer".to_string(),
            audience: vec!["audience".to_string()],
            raw: serde_json::json!({}),
        };
        let auth = AuthContext {
            claims,
            token: "test-token".into(),
        };

        let result = list_inventory(State(state), auth, HeaderMap::new(), Query(InventoryQueryParams::default())).await;
        let (status, _) = result.expect_err("missing header should fail");
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
