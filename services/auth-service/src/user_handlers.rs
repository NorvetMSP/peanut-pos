use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{
        header::{COOKIE, SET_COOKIE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use common_auth::AuthContext;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, Postgres, QueryBuilder};
use tracing::{error, info, warn, instrument, Span};
use uuid::Uuid;

use crate::config::AuthConfig;
use crate::mfa::{normalize_mfa_code, verify_totp_code};
use crate::notifications::MfaActivityEvent;
use crate::tokens::{IssuedTokens, TokenSubject};
use crate::AppState;

pub(crate) const ALLOWED_ROLES: &[&str] = &["super_admin", "admin", "manager", "cashier"];

const MAX_FAILED_ATTEMPTS: i16 = 5;
const LOCKOUT_MINUTES: i64 = 15;

const MAX_MFA_FAILED_ATTEMPTS: i16 = 5;
const MFA_LOCKOUT_MINUTES: i64 = 15;

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

    pub(crate) fn account_locked(until: Option<DateTime<Utc>>) -> Self {
        let locked_until = until.map(|time| time.to_rfc3339_opts(SecondsFormat::Secs, true));
        let mut error = Self::new(
            StatusCode::LOCKED,
            "ACCOUNT_LOCKED",
            "This account is locked. Please contact a manager for assistance.",
        );
        error.body.locked_until = locked_until;
        error
    }

    pub(crate) fn account_inactive() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "ACCOUNT_INACTIVE",
            "This account is disabled. Please contact an administrator.",
        )
    }

    pub(crate) fn mfa_required() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "MFA_REQUIRED",
            "Multi-factor authentication is required for this account.",
        )
    }

    pub(crate) fn mfa_not_enrolled() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "MFA_NOT_ENROLLED",
            "MFA is not enrolled for this account. Please complete enrollment.",
        )
    }

    pub(crate) fn mfa_invalid() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "MFA_CODE_INVALID",
            "Invalid MFA code. Please try again.",
        )
    }

    pub(crate) fn session_expired() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "SESSION_EXPIRED",
            "Your session has expired. Please sign in again.",
        )
    }

    pub(crate) fn internal_error(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "SERVER_ERROR", message)
    }

    pub(crate) fn with_detail(
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

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub name: Option<String>,
    pub role: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_password_reset: Option<DateTime<Utc>>,
    pub force_password_reset: bool,
}
#[derive(FromRow)]
#[allow(dead_code)]
struct AuthRow {
    id: Uuid,
    tenant_id: Uuid,
    name: String,
    email: String,
    role: String,
    is_active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    password_hash: String,
    failed_attempts: i16,
    locked_until: Option<DateTime<Utc>>,
    last_password_reset: Option<DateTime<Utc>>,
    force_password_reset: bool,
    mfa_secret: Option<String>,
    mfa_pending_secret: Option<String>,
    mfa_enrolled_at: Option<DateTime<Utc>>,
    mfa_failed_attempts: i16,
    mfa_last_challenge_at: Option<DateTime<Utc>>,
}

struct LoginMetadata {
    ip: Option<String>,
    user_agent: Option<String>,
    device_fingerprint: Option<String>,
}

fn build_refresh_cookie(config: &AuthConfig, token: &str, max_age_seconds: i64) -> String {
    let mut parts = Vec::new();
    parts.push(format!("{}={}", config.refresh_cookie_name, token));
    parts.push("Path=/".to_string());
    parts.push("HttpOnly".to_string());

    let max_age = max_age_seconds.max(0);
    parts.push(format!("Max-Age={}", max_age));

    if max_age > 0 {
        let expires = (Utc::now() + Duration::seconds(max_age)).to_rfc2822();
        parts.push(format!("Expires={}", expires));
    }

    if let Some(domain) = &config.refresh_cookie_domain {
        if !domain.is_empty() {
            parts.push(format!("Domain={}", domain));
        }
    }

    parts.push(format!(
        "SameSite={}",
        config.refresh_cookie_same_site.as_str()
    ));
    if config.refresh_cookie_secure {
        parts.push("Secure".to_string());
    }

    parts.join("; ")
}

fn clear_refresh_cookie(config: &AuthConfig) -> String {
    let mut parts = Vec::new();
    parts.push(format!("{}=", config.refresh_cookie_name));
    parts.push("Path=/".to_string());
    parts.push("Max-Age=0".to_string());
    parts.push("Expires=Thu, 01 Jan 1970 00:00:00 GMT".to_string());
    parts.push("HttpOnly".to_string());
    parts.push(format!(
        "SameSite={}",
        config.refresh_cookie_same_site.as_str()
    ));
    if let Some(domain) = &config.refresh_cookie_domain {
        if !domain.is_empty() {
            parts.push(format!("Domain={}", domain));
        }
    }
    if config.refresh_cookie_secure {
        parts.push("Secure".to_string());
    }
    parts.join("; ")
}

fn extract_refresh_cookie(headers: &HeaderMap, config: &AuthConfig) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    let prefix = format!("{}=", config.refresh_cookie_name);
    raw.split(';')
        .map(|segment| segment.trim())
        .find_map(|segment| segment.strip_prefix(&prefix))
        .map(|value| value.to_string())
}

impl LoginMetadata {
    fn from_headers(headers: &HeaderMap, device_fingerprint: Option<String>) -> Self {
        let ip = headers
            .get("x-forwarded-for")
            .or_else(|| headers.get("x-real-ip"))
            .and_then(|value| value.to_str().ok())
            .and_then(|raw| raw.split(',').next().map(|part| part.trim().to_string()))
            .filter(|value| !value.is_empty());

        let user_agent = headers
            .get("user-agent")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        Self {
            ip,
            user_agent,
            device_fingerprint: device_fingerprint.filter(|value| !value.trim().is_empty()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn record_mfa_event(
    state: &AppState,
    action: &'static str,
    severity: &'static str,
    user: &AuthRow,
    metadata: &LoginMetadata,
    trace_id: Uuid,
    detail: Option<String>,
    notify_webhook: bool,
) {
    state.record_mfa_metric(action);
    let ip = metadata.ip.clone();
    let user_agent = metadata.user_agent.clone();
    let device = metadata.device_fingerprint.clone();

    match severity {
        "warn" | "error" => warn!(
            security_event = %action,
            severity,
            user_id = %user.id,
            tenant_id = %user.tenant_id,
            role = %user.role,
            ip = ip.as_deref().unwrap_or(""),
            user_agent = user_agent.as_deref().unwrap_or(""),
            device = device.as_deref().unwrap_or(""),
            trace_id = %trace_id,
            "Recorded MFA activity"
        ),
        _ => info!(
            security_event = %action,
            severity,
            user_id = %user.id,
            tenant_id = %user.tenant_id,
            role = %user.role,
            ip = ip.as_deref().unwrap_or(""),
            user_agent = user_agent.as_deref().unwrap_or(""),
            device = device.as_deref().unwrap_or(""),
            trace_id = %trace_id,
            "Recorded MFA activity"
        ),
    }

    let detail = detail.or_else(|| {
        if notify_webhook {
            Some(
                json!({
                    "ip": metadata.ip.as_deref(),
                    "user_agent": metadata.user_agent.as_deref(),
                    "device": metadata.device_fingerprint.as_deref(),
                })
                .to_string(),
            )
        } else {
            None
        }
    });

    let event = MfaActivityEvent {
        action,
        severity,
        tenant_id: user.tenant_id,
        user_id: Some(user.id),
        trace_id,
        occurred_at: Utc::now(),
        ip,
        user_agent,
        device,
        role: Some(user.role.clone()),
        detail,
    };

    let webhook_message = if notify_webhook {
        let ip = metadata.ip.as_deref().unwrap_or("unknown");
        let device = metadata.device_fingerprint.as_deref().unwrap_or("unknown");
        Some(format!(
            ":rotating_light: {action} detected for {} (tenant {}) trace {trace_id} ip {ip} device {device}",
            user.email, user.tenant_id
        ))
    } else {
        None
    };

    state.emit_mfa_activity(event, webhook_message).await;
}

pub async fn create_user(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Json(new_user): Json<NewUser>,
) -> Result<Json<User>, (StatusCode, String)> {
    ensure_role_any(&auth, &["super_admin", "admin"])?;
    let tenant_id = extract_tenant_id(&headers)?;
    ensure_tenant_access(&auth, tenant_id)?;

    let user_id = Uuid::new_v4();
    let NewUser {
        name,
        email,
        password,
        role,
    } = new_user;

    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Name must not be empty".to_string(),
        ));
    }

    let trimmed_email = email.trim();
    if trimmed_email.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Email must not be empty".to_string(),
        ));
    }

    let trimmed_role = role.trim();
    validate_role(trimmed_role)?;
    let password_hash = hash_password(&password)?;
    let now = Utc::now();

    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (id, tenant_id, name, email, role, password_hash, is_active, created_at, updated_at, last_password_reset, force_password_reset)
         VALUES ($1, $2, $3, $4, $5, $6, TRUE, $7, $8, NULL, TRUE)
         RETURNING id, tenant_id, name, email, role, is_active, created_at, updated_at, last_password_reset, force_password_reset",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(trimmed_name)
    .bind(trimmed_email)
    .bind(trimmed_role)
    .bind(password_hash)
    .bind(now)
    .bind(now)
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
    auth: AuthContext,
    headers: HeaderMap,
) -> Result<Json<Vec<User>>, (StatusCode, String)> {
    ensure_role_any(&auth, &["super_admin", "admin", "manager"])?;
    let tenant_id = extract_tenant_id(&headers)?;
    ensure_tenant_access(&auth, tenant_id)?;

    let users = sqlx::query_as::<_, User>(
        "SELECT id, tenant_id, name, email, role, is_active, created_at, updated_at, last_password_reset, force_password_reset
         FROM users
         WHERE tenant_id = $1
         ORDER BY name",
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

pub async fn update_user(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
    Json(payload): Json<UpdateUserRequest>,
) -> Result<Json<User>, (StatusCode, String)> {
    ensure_role_any(&auth, &["super_admin", "admin"])?;
    let tenant_id = extract_tenant_id(&headers)?;
    ensure_tenant_access(&auth, tenant_id)?;

    let existing = sqlx::query_as::<_, User>(
        "SELECT id, tenant_id, name, email, role, is_active, created_at, updated_at, last_password_reset, force_password_reset
         FROM users
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(user_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {e}")))?
    .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let mut name_changed = false;
    let mut role_changed = false;
    let mut active_changed = false;
    let mut updated_name = existing.name.clone();
    let mut updated_role = existing.role.clone();
    let mut updated_active = existing.is_active;

    if let Some(name) = payload.name.as_ref() {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "Name must not be empty".to_string(),
            ));
        }
        if trimmed != existing.name {
            updated_name = trimmed.to_string();
            name_changed = true;
        }
    }
    if let Some(role) = payload.role.as_ref() {
        let trimmed = role.trim();
        if trimmed.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "Role must not be empty".to_string(),
            ));
        }
        validate_role(trimmed)?;
        if trimmed != existing.role {
            updated_role = trimmed.to_string();
            role_changed = true;
        }
    }

    if let Some(is_active) = payload.is_active {
        if is_active != existing.is_active {
            updated_active = is_active;
            active_changed = true;
        }
    }

    if !name_changed && !role_changed && !active_changed {
        return Ok(Json(existing));
    }

    let now = Utc::now();
    let mut builder = QueryBuilder::<Postgres>::new("UPDATE users SET ");
    {
        let mut separated = builder.separated(", ");
        separated.push("updated_at = ");
        separated.push_bind(now);
        if name_changed {
            separated.push("name = ");
            separated.push_bind(&updated_name);
        }
        if role_changed {
            separated.push("role = ");
            separated.push_bind(&updated_role);
        }
        if active_changed {
            separated.push("is_active = ");
            separated.push_bind(updated_active);
        }
    }
    builder.push(" WHERE id = ");
    builder.push_bind(user_id);
    builder.push(" AND tenant_id = ");
    builder.push_bind(tenant_id);
    builder.push(
        " RETURNING id, tenant_id, name, email, role, is_active, created_at, updated_at, last_password_reset, force_password_reset",
    );

    let user = builder
        .build_query_as::<User>()
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

pub async fn reset_user_password(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
    Json(payload): Json<ResetPasswordRequest>,
) -> Result<Json<User>, (StatusCode, String)> {
    ensure_role_any(&auth, &["super_admin", "admin"])?;
    let tenant_id = extract_tenant_id(&headers)?;
    ensure_tenant_access(&auth, tenant_id)?;

    let trimmed = payload.password.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Password must not be empty".to_string(),
        ));
    }
    if trimmed.len() < 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Password must be at least 8 characters long.".to_string(),
        ));
    }

    let password_hash = hash_password(trimmed)?;
    let now = Utc::now();

    let user = sqlx::query_as::<_, User>(
        "UPDATE users
         SET password_hash = $1, updated_at = $2, last_password_reset = $2, force_password_reset = TRUE
         WHERE id = $3 AND tenant_id = $4
         RETURNING id, tenant_id, name, email, role, is_active, created_at, updated_at, last_password_reset, force_password_reset",
    )
    .bind(password_hash)
    .bind(now)
    .bind(user_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {e}")))?
    .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    Ok(Json(user))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    #[serde(default, alias = "tenantId")]
    pub tenant_id: Option<Uuid>,
    #[serde(default, alias = "mfaCode")]
    pub mfa_code: Option<String>,
    #[serde(default, alias = "deviceFingerprint")]
    pub device_fingerprint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// Retained for backward compatibility with existing clients.
    pub token: String,
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub refresh_expires_in: i64,
    pub token_type: &'static str,
    pub access_token_expires_at: String,
    pub refresh_token_expires_at: String,
    pub user: User,
}

pub async fn login_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(login): Json<LoginRequest>,
) -> Result<Response, AuthError> {
    let LoginRequest {
        email,
        password,
        tenant_id,
        mfa_code,
        device_fingerprint,
    } = login;

    let metadata = LoginMetadata::from_headers(&headers, device_fingerprint);
    let trace_id = Uuid::new_v4();
    state.record_login_metric("attempt");

    let user_row = match tenant_id {
        Some(tenant) => sqlx::query_as::<_, AuthRow>(
            "SELECT id, tenant_id, name, email, role, is_active, created_at, updated_at, password_hash, failed_attempts, locked_until, last_password_reset, force_password_reset, mfa_secret, mfa_pending_secret, mfa_enrolled_at, mfa_failed_attempts, mfa_last_challenge_at FROM users WHERE email = $1 AND tenant_id = $2",
        )
        .bind(&email)
        .bind(tenant)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AuthError::internal_error(format!("DB query failed: {e}")))?,
        None => sqlx::query_as::<_, AuthRow>(
            "SELECT id, tenant_id, name, email, role, is_active, created_at, updated_at, password_hash, failed_attempts, locked_until, last_password_reset, force_password_reset, mfa_secret, mfa_pending_secret, mfa_enrolled_at, mfa_failed_attempts, mfa_last_challenge_at FROM users WHERE email = $1",
        )
        .bind(&email)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AuthError::internal_error(format!("DB query failed: {e}")))?,
    };

    let mut auth_data = match user_row {
        Some(row) => row,
        None => {
            state.record_login_metric("invalid_credentials");
            return Err(AuthError::invalid_credentials());
        }
    };

    if !auth_data.is_active {
        state.record_login_metric("account_inactive");
        return Err(AuthError::account_inactive());
    }

    let now = Utc::now();

    if let Some(locked_until) = auth_data.locked_until {
        if locked_until > now {
            return Err(AuthError::account_locked(Some(locked_until)));
        }

        if let Err(err) = sqlx::query(
            "UPDATE users SET failed_attempts = 0, mfa_failed_attempts = 0, locked_until = NULL WHERE id = $1",
        )
        .bind(auth_data.id)
        .execute(&state.db)
        .await
        {
            warn!(
                user_id = %auth_data.id,
                error = ?err,
                "Failed to reset expired lockout"
            );
        } else {
            auth_data.failed_attempts = 0;
            auth_data.mfa_failed_attempts = 0;
            auth_data.locked_until = None;
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
                        if let Err(err) = sqlx::query(
                            "UPDATE users SET password_hash = $1 WHERE id = $2",
                        )
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

        if let Err(err) = sqlx::query(
            "UPDATE users SET failed_attempts = $1, locked_until = $2 WHERE id = $3",
        )
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
        if let Err(err) = sqlx::query(
            "UPDATE users SET failed_attempts = 0, locked_until = NULL WHERE id = $1",
        )
        .bind(auth_data.id)
        .execute(&state.db)
        .await
        {
            warn!(
                user_id = %auth_data.id,
                error = ?err,
                "Failed to reset failed attempts after successful login"
            );
        } else {
            auth_data.failed_attempts = 0;
            auth_data.locked_until = None;
        }
    }

    let requires_mfa = state.config.should_enforce_for(
        &auth_data.role,
        auth_data.tenant_id,
        auth_data.mfa_secret.is_some() || auth_data.mfa_pending_secret.is_some(),
    );

    if requires_mfa {
        if auth_data.mfa_pending_secret.is_some() {
            record_mfa_event(
                &state,
                "mfa.challenge.pending_secret",
                "info",
                &auth_data,
                &metadata,
                trace_id,
                Some(
                    json!({
                        "reason": "pending_secret",
                        "ip": metadata.ip.as_deref(),
                        "user_agent": metadata.user_agent.as_deref(),
                        "device": metadata.device_fingerprint.as_deref(),
                    })
                    .to_string(),
                ),
                false,
            )
            .await;
            state.record_login_metric("mfa_required");
            return Err(AuthError::mfa_required());
        }

        let secret = match auth_data.mfa_secret.as_deref() {
            Some(secret) => secret,
            None => {
                record_mfa_event(
                    &state,
                    "mfa.challenge.unenrolled",
                    "warn",
                    &auth_data,
                    &metadata,
                    trace_id,
                    Some(
                        json!({
                            "reason": "unenrolled",
                            "ip": metadata.ip.as_deref(),
                            "user_agent": metadata.user_agent.as_deref(),
                            "device": metadata.device_fingerprint.as_deref(),
                        })
                        .to_string(),
                    ),
                    false,
                )
                .await;
                state.record_login_metric("mfa_not_enrolled");
                return Err(AuthError::mfa_not_enrolled());
            }
        };
        let code = match mfa_code
            .as_deref()
            .and_then(normalize_mfa_code)
        {
            Some(code) => code,
            None => {
                record_mfa_event(
                    &state,
                    "mfa.challenge.missing_code",
                    "warn",
                    &auth_data,
                    &metadata,
                    trace_id,
                    Some(
                        json!({
                            "reason": "missing_code",
                            "ip": metadata.ip.as_deref(),
                            "user_agent": metadata.user_agent.as_deref(),
                            "device": metadata.device_fingerprint.as_deref(),
                        })
                        .to_string(),
                    ),
                    false,
                )
                .await;
                state.record_login_metric("mfa_required");
                return Err(AuthError::mfa_required());
            }
        };

        if !verify_totp_code(secret, &code) {
            let challenge_at = Utc::now();
            let next_failed = auth_data.mfa_failed_attempts.saturating_add(1);
            record_mfa_event(
                &state,
                "mfa.challenge.failed",
                "warn",
                &auth_data,
                &metadata,
                trace_id,
                Some(
                    json!({
                        "reason": "invalid_code",
                        "ip": metadata.ip.as_deref(),
                        "user_agent": metadata.user_agent.as_deref(),
                        "device": metadata.device_fingerprint.as_deref(),
                    })
                    .to_string(),
                ),
                true,
            )
            .await;

            if next_failed >= MAX_MFA_FAILED_ATTEMPTS {
                let lock_until = challenge_at + Duration::minutes(MFA_LOCKOUT_MINUTES);
                if let Err(err) = sqlx::query(
                    "UPDATE users SET mfa_failed_attempts = 0, locked_until = $2, mfa_last_challenge_at = $3 WHERE id = $1",
                )
                .bind(auth_data.id)
                .bind(lock_until)
                .bind(challenge_at)
                .execute(&state.db)
                .await
                {
                    warn!(
                        user_id = %auth_data.id,
                        error = ?err,
                        "Failed to persist MFA lockout"
                    );
                }
                record_mfa_event(
                    &state,
                    "mfa.challenge.lockout",
                    "error",
                    &auth_data,
                    &metadata,
                    trace_id,
                    Some(
                        json!({
                            "reason": "lockout",
                            "ip": metadata.ip.as_deref(),
                            "user_agent": metadata.user_agent.as_deref(),
                            "device": metadata.device_fingerprint.as_deref(),
                        })
                        .to_string(),
                    ),
                    true,
                )
                .await;
                state.record_login_metric("mfa_lockout");
                return Err(AuthError::account_locked(Some(lock_until)));
            } else {
                if let Err(err) = sqlx::query(
                    "UPDATE users SET mfa_failed_attempts = $2, mfa_last_challenge_at = $3 WHERE id = $1",
                )
                .bind(auth_data.id)
                .bind(next_failed)
                .bind(challenge_at)
                .execute(&state.db)
                .await
                {
                    warn!(
                        user_id = %auth_data.id,
                        error = ?err,
                        "Failed to record MFA failure"
                    );
                } else {
                    auth_data.mfa_failed_attempts = next_failed;
                }

                state.record_login_metric("mfa_invalid");
                return Err(AuthError::mfa_invalid());
            }
        }

        if let Err(err) = sqlx::query(
            "UPDATE users SET mfa_failed_attempts = 0, mfa_last_challenge_at = NOW() WHERE id = $1",
        )
        .bind(auth_data.id)
        .execute(&state.db)
        .await
        {
            warn!(
                user_id = %auth_data.id,
                error = ?err,
                "Failed to reset MFA counters after success"
            );
        } else {
            auth_data.mfa_failed_attempts = 0;
        }

        record_mfa_event(
            &state,
            "mfa.challenge.succeeded",
            "info",
            &auth_data,
            &metadata,
            trace_id,
            Some(
                json!({
                    "reason": "success",
                    "ip": metadata.ip.as_deref(),
                    "user_agent": metadata.user_agent.as_deref(),
                    "device": metadata.device_fingerprint.as_deref(),
                })
                .to_string(),
            ),
            false,
        )
        .await;
    }

    let user = User {
        id: auth_data.id,
        tenant_id: auth_data.tenant_id,
        name: auth_data.name,
        email: auth_data.email,
        role: auth_data.role.clone(),
        is_active: auth_data.is_active,
        created_at: auth_data.created_at,
        updated_at: auth_data.updated_at,
        last_password_reset: auth_data.last_password_reset,
        force_password_reset: auth_data.force_password_reset,
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

    let refresh_cookie =
        build_refresh_cookie(state.config.as_ref(), &refresh_token, refresh_expires_in);

    let response = LoginResponse {
        token: access_token.clone(),
        access_token,
        refresh_token: None,
        expires_in: access_expires_in,
        refresh_expires_in,
        token_type,
        access_token_expires_at: access_expires_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        refresh_token_expires_at: refresh_expires_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        user,
    };

    state.record_login_metric("success");

    let mut reply = Json(response).into_response();
    match HeaderValue::from_str(&refresh_cookie) {
        Ok(value) => {
            reply.headers_mut().append(SET_COOKIE, value);
        }
        Err(err) => {
            error!(error = ?err, "Failed to encode refresh cookie");
            return Err(AuthError::internal_error(
                "Unable to encode refresh cookie.",
            ));
        }
    }

    Ok(reply)
}

#[instrument(name = "refresh_session", skip(state, headers), fields(outcome = "pending"))]
pub async fn refresh_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    let Some(raw_cookie) = extract_refresh_cookie(&headers, state.config.as_ref()) else {
        tracing::debug!("missing refresh cookie");
    Span::current().record("outcome", tracing::field::display("missing_cookie"));
        return Err(AuthError::session_expired());
    };

    let consumed = state.token_signer.consume_refresh_token(&raw_cookie).await;
    let consumed = match consumed {
        Ok(c) => c,
        Err(err) => {
            // Distinguish between an invalid/expired token vs infrastructure error.
            // If token parsing/validation failed we surface session_expired (401) instead of 500.
            let err_str = err.to_string();
            if err_str.contains("expired") || err_str.contains("invalid") {
                warn!(error = %err, "refresh token invalid or expired");
                Span::current().record("outcome", tracing::field::display("invalid_refresh"));
                return Err(AuthError::session_expired());
            }
            error!(error = %err, "Failed to consume refresh token (infrastructure error)");
            Span::current().record("outcome", tracing::field::display("error"));
            return Err(AuthError::internal_error("Unable to refresh session."));
        }
    };

    let account = match consumed {
        Some(account) => account,
        None => {
            Span::current().record("outcome", tracing::field::display("not_found"));
            return Err(AuthError::session_expired());
        }
    };

    let user = User {
        id: account.user_id,
        tenant_id: account.tenant_id,
        name: account.name,
        email: account.email,
        role: account.role.clone(),
        is_active: account.is_active,
        created_at: account.created_at,
        updated_at: account.updated_at,
        last_password_reset: account.last_password_reset,
        force_password_reset: account.force_password_reset,
    };

    let subject = TokenSubject {
        user_id: user.id,
        tenant_id: user.tenant_id,
        roles: vec![user.role.clone()],
    };

    let issued = state.token_signer.issue_tokens(subject).await.map_err(|err| {
        error!(user_id = %user.id, error = %err, "Failed to issue tokens during session refresh");
    Span::current().record("outcome", tracing::field::display("issue_error"));
        AuthError::internal_error("Unable to refresh session.")
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

    let refresh_cookie =
        build_refresh_cookie(state.config.as_ref(), &refresh_token, refresh_expires_in);

    let response = LoginResponse {
        token: access_token.clone(),
        access_token,
        refresh_token: None,
        expires_in: access_expires_in,
        refresh_expires_in,
        token_type,
        access_token_expires_at: access_expires_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        refresh_token_expires_at: refresh_expires_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        user,
    };

    let mut reply = Json(response).into_response();
    match HeaderValue::from_str(&refresh_cookie) {
        Ok(value) => {
            reply.headers_mut().append(SET_COOKIE, value);
        }
        Err(err) => {
            error!(error = ?err, "Failed to encode refresh cookie");
            return Err(AuthError::internal_error(
                "Unable to encode refresh cookie.",
            ));
        }
    }

    Span::current().record("outcome", tracing::field::display("success"));
    Ok(reply)
}

pub async fn logout_user(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(raw_cookie) = extract_refresh_cookie(&headers, state.config.as_ref()) {
        if let Err(err) = state.token_signer.consume_refresh_token(&raw_cookie).await {
            warn!(error = %err, "Failed to revoke refresh token during logout");
        }
    }

    let clear_cookie = clear_refresh_cookie(state.config.as_ref());
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;

    match HeaderValue::from_str(&clear_cookie) {
        Ok(value) => {
            response.headers_mut().insert(SET_COOKIE, value);
        }
        Err(err) => {
            error!(error = ?err, "Failed to encode refresh cookie during logout");
        }
    }

    response
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

fn ensure_role_any(auth: &AuthContext, allowed: &[&str]) -> Result<(), (StatusCode, String)> {
    if allowed.iter().any(|role| auth.has_role(role)) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            format!("Insufficient role. Required one of: {}", allowed.join(", ")),
        ))
    }
}

fn ensure_tenant_access(auth: &AuthContext, tenant_id: Uuid) -> Result<(), (StatusCode, String)> {
    if auth.has_role("super_admin") || auth.claims.tenant_id == tenant_id {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "You are not permitted to manage another tenant.".to_string(),
        ))
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
    let roles = ALLOWED_ROLES.to_vec();
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
#[cfg(test)]
mod tests {
    use super::*;
    use argon2::{password_hash::PasswordHash, Argon2};
    use axum::http::{header::COOKIE, HeaderMap, HeaderValue};
    use std::collections::HashSet;

    use crate::config::CookieSameSite;

    fn test_config() -> AuthConfig {
        AuthConfig {
            require_mfa: false,
            required_roles: HashSet::new(),
            bypass_tenants: HashSet::new(),
            mfa_issuer: "NovaPOS".to_string(),
            mfa_activity_topic: "security.mfa.activity".to_string(),
            mfa_dead_letter_topic: None,
            suspicious_webhook_url: None,
            suspicious_webhook_bearer: None,
            refresh_cookie_name: "novapos_refresh".to_string(),
            refresh_cookie_domain: Some("example.com".to_string()),
            refresh_cookie_secure: true,
            refresh_cookie_same_site: CookieSameSite::Strict,
        }
    }

    #[test]
    fn build_refresh_cookie_sets_expected_attributes() {
        let config = test_config();
        let cookie = build_refresh_cookie(&config, "token123", 3600);

        assert!(cookie.contains("novapos_refresh=token123"));
        assert!(cookie.contains("Max-Age=3600"));
        assert!(cookie.contains("Expires="));
        assert!(cookie.contains("Domain=example.com"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn build_refresh_cookie_handles_negative_max_age() {
        let mut config = test_config();
        config.refresh_cookie_domain = None;
        config.refresh_cookie_secure = false;

        let cookie = build_refresh_cookie(&config, "short", -10);
        assert!(cookie.contains("Max-Age=0"));
        assert!(!cookie.contains("Expires="));
        assert!(!cookie.contains("Domain="));
        assert!(!cookie.contains("Secure"));
    }

    #[test]
    fn clear_refresh_cookie_produces_expired_cookie() {
        let config = test_config();
        let cookie = clear_refresh_cookie(&config);
        assert!(cookie.contains("novapos_refresh="));
        assert!(cookie.contains("Max-Age=0"));
        assert!(cookie.contains("Expires=Thu, 01 Jan 1970 00:00:00 GMT"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn extract_refresh_cookie_reads_value() {
        let config = test_config();
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("foo=bar; novapos_refresh=mytoken; other=value"),
        );

        let value = extract_refresh_cookie(&headers, &config);
        assert_eq!(value.as_deref(), Some("mytoken"));
    }

    #[test]
    fn extract_refresh_cookie_handles_missing_header() {
        let config = test_config();
        let headers = HeaderMap::new();
        assert!(extract_refresh_cookie(&headers, &config).is_none());
    }

    #[test]
    fn extract_tenant_id_parses_header() {
        let mut headers = HeaderMap::new();
        let tenant = Uuid::new_v4();
        headers.insert(
            "X-Tenant-ID",
            HeaderValue::from_str(&tenant.to_string()).expect("tenant header"),
        );

        let result = extract_tenant_id(&headers).expect("tenant id");
        assert_eq!(result, tenant);
    }

    #[test]
    fn extract_tenant_id_rejects_invalid_value() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Tenant-ID", HeaderValue::from_static("not-a-uuid"));

        let err = extract_tenant_id(&headers).expect_err("should fail");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("Invalid"));
    }

    #[test]
    fn validate_role_accepts_allowed_roles() {
        for role in ALLOWED_ROLES {
            validate_role(role).expect("role allowed");
        }
    }

    #[test]
    fn validate_role_rejects_unknown_role() {
        let err = validate_role("guest").expect_err("reject guest");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("Unsupported role"));
    }

    #[test]
    fn hash_password_rejects_blank_passwords() {
        let err = hash_password("   ").expect_err("reject blank");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn hash_password_generates_verifiable_hash() {
        let password = "CorrectHorseBatteryStaple!";
        let hashed = hash_password(password).expect("hash");
        assert_ne!(hashed, password);

        let parsed = PasswordHash::new(&hashed).expect("parse hash");
        assert!(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok());
    }
}
