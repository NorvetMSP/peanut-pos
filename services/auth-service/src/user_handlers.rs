use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tracing::{error, warn};
use uuid::Uuid;

use crate::AppState;

pub(crate) const ALLOWED_ROLES: &[&str] = &["super_admin", "admin", "manager", "cashier"];

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

    validate_role(&role)?;
    let password_hash = hash_password(&password)?;

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

    let auth_rec = sqlx::query_as::<_, AuthRow>(
        "SELECT id, tenant_id, name, email, role, password_hash FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("DB query failed: {e}"),
        )
    })?;

    let auth_data = match auth_rec {
        Some(row) => row,
        None => return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".into())),
    };

    let password_valid = match PasswordHash::new(&auth_data.password_hash) {
        Ok(parsed_hash) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok(),
        Err(_) => {
            if auth_data.password_hash == password {
                match hash_password(&password) {
                    Ok(new_hash) => {
                        if let Err(err) =
                            sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
                                .bind(&new_hash)
                                .bind(auth_data.id)
                                .execute(&state.db)
                                .await
                        {
                            warn!(user_id = %auth_data.id, error = ?err, "Failed to upgrade password hash");
                        }
                        true
                    }
                    Err((_, message)) => {
                        error!(user_id = %auth_data.id, message = %message, "Unable to upgrade password hash");
                        false
                    }
                }
            } else {
                false
            }
        }
    };

    if !password_valid {
        return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".into()));
    }

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

pub(crate) fn extract_tenant_id(headers: &HeaderMap) -> Result<Uuid, (StatusCode, String)> {
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

fn validate_role(role: &str) -> Result<(), (StatusCode, String)> {
    if ALLOWED_ROLES.contains(&role) {
        Ok(())
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Unsupported role '{role}'. Allowed roles: {}",
                ALLOWED_ROLES.join(", ")
            ),
        ))
    }
}

pub async fn list_roles() -> Json<Vec<&'static str>> {
    let roles = ALLOWED_ROLES.iter().copied().collect::<Vec<_>>();
    Json(roles)
}

fn hash_password(password: &str) -> Result<String, (StatusCode, String)> {
    if password.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Password must not be empty".to_string(),
        ));
    }

    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to hash password: {err}"),
            )
        })
}
