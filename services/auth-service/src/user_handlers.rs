use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;

#[derive(Deserialize)]
pub struct NewUser {
    pub name: String,
    pub email: String,
    pub password: String,
    pub role: String,
}

#[derive(Serialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: String,
}

#[derive(FromRow)]
struct AuthRow {
    id: Uuid,
    tenant_id: Uuid,
    name: String,
    email: String,
    role: String,
    password_hash: String,
}

pub async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(new_user): Json<NewUser>,
) -> Result<Json<User>, (StatusCode, String)> {
    let tenant_id = extract_tenant_id(&headers)?;

    let user_id = Uuid::new_v4();
    let NewUser {
        name,
        email,
        password,
        role,
    } = new_user;
    let password_hash = password;

    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (id, tenant_id, name, email, role, password_hash)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, tenant_id, name, email, role",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(name)
    .bind(email)
    .bind(role)
    .bind(password_hash)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {e}"),
        )
    })?;

    Ok(Json(user))
}

pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<User>>, (StatusCode, String)> {
    let tenant_id = extract_tenant_id(&headers)?;

    let users = sqlx::query_as::<_, User>(
        "SELECT id, tenant_id, name, email, role FROM users WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {e}"),
        )
    })?;

    Ok(Json(users))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: User,
}

pub async fn login_user(
    State(state): State<AppState>,
    Json(login): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    let LoginRequest { email, password } = login;
    // Find user by email
    let auth_rec = sqlx::query_as::<_, AuthRow>(
        "SELECT id, tenant_id, name, email, role, password_hash FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("DB query failed: {}", e),
        )
    })?;
    let auth_data = match auth_rec {
        Some(row) => row,
        None => return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".into())),
    };
    if auth_data.password_hash != password {
        return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".into()));
    }
    // Generate session token (not stored on server; client will cache for offline use)
    let token = Uuid::new_v4().to_string();
    let user = User {
        id: auth_data.id,
        tenant_id: auth_data.tenant_id,
        name: auth_data.name,
        email: auth_data.email,
        role: auth_data.role,
    };
    Ok(Json(LoginResponse { token, user }))
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
