use axum::{Json, http::{HeaderMap, StatusCode}};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::AppState;
use sqlx::query_as;

#[derive(Deserialize)]
pub struct NewUser {
    pub name: String,
    pub email: String,
    pub password: String,
    pub role: String
}

#[derive(Serialize)]
pub struct User {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: String
}

pub async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(new_user): Json<NewUser>
) -> Result<Json<User>, (StatusCode, String)> {
    // Extract tenant ID
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => return Err((StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID header".to_string()))
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Missing X-Tenant-ID header".to_string()));
    };
    // Generate new user ID
    let user_id = Uuid::new_v4();
    // In a real system, hash the password here
    let password_hash = new_user.password;
    // Insert user into database
    let user = query_as!(
        User,
        "INSERT INTO users (id, tenant_id, name, email, role, password_hash) 
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, tenant_id, name, email, role",
        user_id,
        tenant_id,
        new_user.name,
        new_user.email,
        new_user.role,
        password_hash
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;
    Ok(Json(user))
}

pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap
) -> Result<Json<Vec<User>>, (StatusCode, String)> {
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => return Err((StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID header".to_string()))
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Missing X-Tenant-ID header".to_string()));
    };
    let users = query_as!(
        User,
        "SELECT id, tenant_id, name, email, role FROM users WHERE tenant_id = $1",
        tenant_id
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;
    Ok(Json(users))
}
