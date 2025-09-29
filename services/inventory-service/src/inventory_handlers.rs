use crate::AppState;
use axum::extract::State;
use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};
use common_auth::AuthContext;
use serde::Serialize;
use sqlx::query_as;
use uuid::Uuid;

pub(crate) const LIST_INVENTORY_SQL: &str =
    "SELECT product_id, tenant_id, quantity, threshold FROM inventory WHERE tenant_id = $1";

pub(crate) const INVENTORY_VIEW_ROLES: &[&str] = &["super_admin", "admin", "manager", "cashier"];

pub(crate) fn tenant_id_from_request(
    headers: &HeaderMap,
    auth: &AuthContext,
) -> Result<Uuid, (StatusCode, String)> {
    let header_value = headers
        .get("X-Tenant-ID")
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing X-Tenant-ID header".to_string(),
        ))?
        .to_str()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid X-Tenant-ID header".to_string(),
            )
        })?
        .trim();
    let tenant_id = Uuid::parse_str(header_value).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid X-Tenant-ID header".to_string(),
        )
    })?;
    if tenant_id != auth.claims.tenant_id {
        return Err((
            StatusCode::FORBIDDEN,
            "Authenticated tenant does not match X-Tenant-ID header".to_string(),
        ));
    }
    Ok(tenant_id)
}

pub(crate) fn ensure_role(
    auth: &AuthContext,
    allowed: &[&str],
) -> Result<(), (StatusCode, String)> {
    let has_role = auth
        .claims
        .roles
        .iter()
        .any(|role| allowed.iter().any(|required| role == required));
    if has_role {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            format!("Insufficient role. Required one of: {}", allowed.join(", ")),
        ))
    }
}

#[derive(Debug, sqlx::FromRow, Serialize)]
pub struct InventoryRecord {
    pub product_id: Uuid,
    pub tenant_id: Uuid,
    pub quantity: i32,
    pub threshold: i32,
}

pub async fn list_inventory(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
) -> Result<Json<Vec<InventoryRecord>>, (StatusCode, String)> {
    ensure_role(&auth, INVENTORY_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

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

        let result = list_inventory(State(state), auth, HeaderMap::new()).await;
        let (status, _) = result.expect_err("missing header should fail");
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
