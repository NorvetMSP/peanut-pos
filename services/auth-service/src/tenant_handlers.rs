use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{user_handlers::extract_tenant_id, AppState};

const ROOT_TENANT_ID: Uuid = Uuid::from_u128(1);

#[derive(Deserialize)]
pub struct NewTenant {
    pub name: String,
}

#[derive(Serialize, FromRow)]
pub struct TenantRow {
    pub id: Uuid,
    pub name: String,
}

#[derive(Deserialize)]
pub struct NewIntegrationKey {
    pub label: Option<String>,
    pub revoke_existing: Option<bool>,
}

#[derive(Serialize, FromRow)]
pub struct IntegrationKeyRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub label: String,
    pub key_suffix: String,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub struct IntegrationKeyCreated {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub label: String,
    pub key_suffix: String,
    pub created_at: DateTime<Utc>,
    pub api_key: String,
}

pub async fn create_tenant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<NewTenant>,
) -> Result<Json<TenantRow>, (StatusCode, String)> {
    ensure_super_admin(&headers)?;
    let name = payload.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Tenant name is required".into()));
    }

    let tenant_id = Uuid::new_v4();
    let tenant = sqlx::query_as::<_, TenantRow>(
        "INSERT INTO tenants (id, name) VALUES ($1, $2) RETURNING id, name",
    )
    .bind(tenant_id)
    .bind(name)
    .fetch_one(&state.db)
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create tenant: {err}"),
        )
    })?;

    Ok(Json(tenant))
}

pub async fn list_tenants(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<TenantRow>>, (StatusCode, String)> {
    ensure_super_admin(&headers)?;
    let tenants = sqlx::query_as::<_, TenantRow>("SELECT id, name FROM tenants ORDER BY name")
        .fetch_all(&state.db)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load tenants: {err}"),
            )
        })?;

    Ok(Json(tenants))
}

pub async fn list_integration_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<Vec<IntegrationKeyRow>>, (StatusCode, String)> {
    ensure_tenant_scope(&headers, tenant_id)?;
    let keys = sqlx::query_as::<_, IntegrationKeyRow>(
        "SELECT id, tenant_id, label, key_suffix, created_at, revoked_at
         FROM integration_keys
         WHERE tenant_id = $1
         ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.db)
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load integration keys: {err}"),
        )
    })?;

    Ok(Json(keys))
}

pub async fn create_integration_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(tenant_id): Path<Uuid>,
    Json(payload): Json<NewIntegrationKey>,
) -> Result<Json<IntegrationKeyCreated>, (StatusCode, String)> {
    ensure_tenant_scope(&headers, tenant_id)?;

    let label = payload
        .label
        .unwrap_or_else(|| "Primary".to_string())
        .trim()
        .to_string();
    if label.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Label is required".into()));
    }

    if payload.revoke_existing.unwrap_or(false) {
        if let Err(err) = sqlx::query(
            "UPDATE integration_keys SET revoked_at = NOW() WHERE tenant_id = $1 AND revoked_at IS NULL",
        )
        .bind(tenant_id)
        .execute(&state.db)
        .await
        {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to revoke existing keys: {err}"),
            ));
        }
    }

    let raw_key = generate_api_key();
    let key_hash = hash_api_key(&raw_key);
    let key_suffix = raw_key
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let key_id = Uuid::new_v4();

    let record = sqlx::query_as::<_, IntegrationKeyRow>(
        "INSERT INTO integration_keys (id, tenant_id, label, api_key_hash, key_suffix)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, tenant_id, label, key_suffix, created_at, revoked_at",
    )
    .bind(key_id)
    .bind(tenant_id)
    .bind(&label)
    .bind(&key_hash)
    .bind(&key_suffix)
    .fetch_one(&state.db)
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create integration key: {err}"),
        )
    })?;

    Ok(Json(IntegrationKeyCreated {
        id: record.id,
        tenant_id: record.tenant_id,
        label: record.label,
        key_suffix: record.key_suffix,
        created_at: record.created_at,
        api_key: raw_key,
    }))
}

pub async fn revoke_integration_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<Uuid>,
) -> Result<Json<IntegrationKeyRow>, (StatusCode, String)> {
    let caller_tenant = extract_tenant_id(&headers)?;

    let existing = sqlx::query_as::<_, IntegrationKeyRow>(
        "SELECT id, tenant_id, label, key_suffix, created_at, revoked_at
         FROM integration_keys WHERE id = $1",
    )
    .bind(key_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load integration key: {err}"),
        )
    })?;

    let record = match existing {
        Some(row) => row,
        None => return Err((StatusCode::NOT_FOUND, "Integration key not found".into())),
    };

    if caller_tenant != ROOT_TENANT_ID && caller_tenant != record.tenant_id {
        return Err((
            StatusCode::FORBIDDEN,
            "Not authorized to revoke this key".into(),
        ));
    }

    if record.revoked_at.is_some() {
        return Err((
            StatusCode::CONFLICT,
            "Integration key already revoked".into(),
        ));
    }

    let updated = sqlx::query_as::<_, IntegrationKeyRow>(
        "UPDATE integration_keys
         SET revoked_at = NOW()
         WHERE id = $1 AND revoked_at IS NULL
         RETURNING id, tenant_id, label, key_suffix, created_at, revoked_at",
    )
    .bind(key_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to revoke integration key: {err}"),
        )
    })?;

    match updated {
        Some(row) => Ok(Json(row)),
        None => Err((
            StatusCode::CONFLICT,
            "Integration key already revoked".into(),
        )),
    }
}

fn ensure_super_admin(headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    let tenant = extract_tenant_id(headers)?;
    if tenant == ROOT_TENANT_ID {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "Super admin tenant required".into()))
    }
}

fn ensure_tenant_scope(headers: &HeaderMap, target: Uuid) -> Result<(), (StatusCode, String)> {
    let tenant = extract_tenant_id(headers)?;
    if tenant == ROOT_TENANT_ID || tenant == target {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "Insufficient privileges for tenant".into(),
        ))
    }
}

fn generate_api_key() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}
