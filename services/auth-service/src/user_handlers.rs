use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tracing::{error, warn};
use uuid::Uuid;

use crate::tokens::{IssuedTokens, TokenSubject};
use crate::AppState;

pub(crate) const ALLOWED_ROLES: &[&str] = &["super_admin", "admin", "manager", "cashier"];

const MAX_FAILED_ATTEMPTS: i16 = 5;
const LOCKOUT_MINUTES: i64 = 15;

#[derive(Debug, Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    locked_until: Option<String>,
}

#[derive(Debug)]
pub struct AuthError {
    status: StatusCode,
    body: ErrorResponse,
}

impl AuthError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ErrorResponse {
                code,
                message: message.into(),
                locked_until: None,
            },
        }
    }

    fn invalid_credentials() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "INVALID_CREDENTIALS",
            "Invalid credentials. Please try again.",
        )
    }

    fn account_locked(until: Option<DateTime<Utc>>) -> Self {
        let locked_until = until.map(|time| time.to_rfc3339_opts(SecondsFormat::Secs, true));
        let mut error = Self::new(
            StatusCode::LOCKED,
            "ACCOUNT_LOCKED",
            "This account is locked. Please contact a manager for assistance.",
        );
        error.body.locked_until = locked_until;
        error
    }

    fn internal_error(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "SERVER_ERROR", message)
    }

    fn with_detail(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
        locked_until: Option<DateTime<Utc>>,
    ) -> Self {
        let mut err = Self::new(status, code, message);
        err.body.locked_until =
            locked_until.map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true));
        err
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

#[derive(Deserialize)]
pub struct NewUser {
    pub name: String,
    pub email: String,
    pub password: String,
    pub role: String,
}

#[derive(Debug, Serialize, FromRow)]
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
    failed_attempts: i16,
    locked_until: Option<DateTime<Utc>>,
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

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// Retained for backward compatibility with existing clients.
    pub token: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub refresh_expires_in: i64,
    pub token_type: &'static str,
    pub access_token_expires_at: String,
    pub refresh_token_expires_at: String,
    pub user: User,
}

pub async fn login_user(
    State(state): State<AppState>,
    Json(login): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AuthError> {
    let LoginRequest { email, password } = login;

    let mut auth_data = match sqlx::query_as::<_, AuthRow>(
        "SELECT id, tenant_id, name, email, role, password_hash, failed_attempts, locked_until FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AuthError::internal_error(format!("DB query failed: {e}")))? {
        Some(row) => row,
        None => return Err(AuthError::invalid_credentials()),
    };

    let now = Utc::now();

    if let Some(locked_until) = auth_data.locked_until {
        if locked_until > now {
            return Err(AuthError::account_locked(Some(locked_until)));
        }

        if auth_data.failed_attempts >= MAX_FAILED_ATTEMPTS {
            if let Err(err) = sqlx::query(
                "UPDATE users SET failed_attempts = 0, locked_until = NULL WHERE id = $1",
            )
            .bind(auth_data.id)
            .execute(&state.db)
            .await
            {
                warn!(user_id = %auth_data.id, error = ?err, "Failed to reset expired lockout");
            } else {
                auth_data.failed_attempts = 0;
                auth_data.locked_until = None;
            }
        }
    }

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
                            warn!(
                                user_id = %auth_data.id,
                                error = ?err,
                                "Failed to upgrade password hash"
                            );
                        }
                        true
                    }
                    Err((status, message)) => {
                        error!(
                            user_id = %auth_data.id,
                            message = %message,
                            "Unable to upgrade password hash"
                        );
                        return Err(AuthError::with_detail(
                            status,
                            "PASSWORD_UPGRADE_FAILED",
                            message,
                            None,
                        ));
                    }
                }
            } else {
                false
            }
        }
    };

    if !password_valid {
        let new_attempts = auth_data.failed_attempts.saturating_add(1);
        let lock_until = if new_attempts >= MAX_FAILED_ATTEMPTS {
            Some(now + Duration::minutes(LOCKOUT_MINUTES))
        } else {
            None
        };

        if let Err(err) =
            sqlx::query("UPDATE users SET failed_attempts = $1, locked_until = $2 WHERE id = $3")
                .bind(new_attempts)
                .bind(lock_until)
                .bind(auth_data.id)
                .execute(&state.db)
                .await
        {
            warn!(
                user_id = %auth_data.id,
                error = ?err,
                "Failed to record failed login attempt"
            );
        }

        if let Some(until) = lock_until {
            return Err(AuthError::account_locked(Some(until)));
        }

        return Err(AuthError::invalid_credentials());
    }

    if auth_data.failed_attempts != 0 || auth_data.locked_until.is_some() {
        if let Err(err) =
            sqlx::query("UPDATE users SET failed_attempts = 0, locked_until = NULL WHERE id = $1")
                .bind(auth_data.id)
                .execute(&state.db)
                .await
        {
            warn!(
                user_id = %auth_data.id,
                error = ?err,
                "Failed to reset failed attempts after successful login"
            );
        }
    }

    let user = User {
        id: auth_data.id,
        tenant_id: auth_data.tenant_id,
        name: auth_data.name,
        email: auth_data.email,
        role: auth_data.role,
    };

    let subject = TokenSubject {
        user_id: user.id,
        tenant_id: user.tenant_id,
        roles: vec![user.role.clone()],
    };

    let issued = state
        .token_signer
        .issue_tokens(subject)
        .await
        .map_err(|err| {
            error!(user_id = %user.id, error = ?err, "Failed to issue tokens");
            AuthError::internal_error("Unable to issue authentication tokens.")
        })?;

    let IssuedTokens {
        access_token,
        refresh_token,
        access_expires_at,
        refresh_expires_at,
        access_expires_in,
        refresh_expires_in,
        token_type,
    } = issued;

    let response = LoginResponse {
        token: access_token.clone(),
        access_token,
        refresh_token,
        expires_in: access_expires_in,
        refresh_expires_in,
        token_type,
        access_token_expires_at: access_expires_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        refresh_token_expires_at: refresh_expires_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        user,
    };

    Ok(Json(response))
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
