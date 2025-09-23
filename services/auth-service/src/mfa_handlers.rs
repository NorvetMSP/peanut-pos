use crate::notifications::MfaActivityEvent;
use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use common_auth::AuthContext;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::mfa::{build_otpauth_uri, generate_totp_secret, normalize_mfa_code, verify_totp_code};
use crate::user_handlers::AuthError;
use crate::AppState;

#[derive(FromRow)]
#[allow(dead_code)]
struct UserMfaState {
    email: String,
    mfa_secret: Option<String>,
    mfa_pending_secret: Option<String>,
}

#[derive(FromRow)]
struct PendingSecret {
    mfa_pending_secret: Option<String>,
}

#[derive(Serialize)]
pub struct MfaEnrollmentResponse {
    pub secret: String,
    pub otpauth_url: String,
    pub already_enrolled: bool,
}

#[derive(Deserialize)]
pub struct MfaVerifyRequest {
    pub code: String,
}

#[derive(Serialize)]
pub struct MfaVerifyResponse {
    pub enabled: bool,
}

pub async fn begin_mfa_enrollment(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<MfaEnrollmentResponse>, AuthError> {
    let user_id = auth.claims.subject;
    let tenant_id = auth.claims.tenant_id;
    let trace_id = Uuid::new_v4();

    let current = sqlx::query_as::<_, UserMfaState>(
        "SELECT email, mfa_secret, mfa_pending_secret FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|err| {
        error!(user_id = %user_id, error = %err, "Failed to load MFA state");
        AuthError::internal_error("Unable to load MFA state")
    })?
    .ok_or_else(|| AuthError::internal_error("User account missing"))?;

    let secret = generate_totp_secret();
    let account_label = format!("{} ({tenant_id})", current.email);
    let otpauth_url = build_otpauth_uri(&state.config.mfa_issuer, &account_label, &secret);

    sqlx::query("UPDATE users SET mfa_pending_secret = $1, mfa_enrolled_at = NULL WHERE id = $2")
        .bind(&secret)
        .bind(user_id)
        .execute(&state.db)
        .await
        .map_err(|err| {
            error!(user_id = %user_id, error = %err, "Failed to persist MFA pending secret");
            AuthError::internal_error("Unable to start MFA enrollment")
        })?;

    info!(
        event = "mfa.enrollment.start",
        user_id = %user_id,
        tenant_id = %tenant_id,
        already_enrolled = current.mfa_secret.is_some(),
        trace_id = %trace_id,
        "Issued new MFA enrollment secret"
    );

    let event = MfaActivityEvent {
        action: "mfa.enrollment.start",
        severity: "info",
        tenant_id,
        user_id: Some(user_id),
        trace_id,
        occurred_at: Utc::now(),
        ip: None,
        user_agent: None,
        device: None,
        role: None,
        detail: Some(
            json!({
                "already_enrolled": current.mfa_secret.is_some(),
            })
            .to_string(),
        ),
    };
    state.emit_mfa_activity(event, None).await;

    Ok(Json(MfaEnrollmentResponse {
        secret,
        otpauth_url,
        already_enrolled: current.mfa_secret.is_some(),
    }))
}

pub async fn verify_mfa_enrollment(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<MfaVerifyRequest>,
) -> Result<Json<MfaVerifyResponse>, AuthError> {
    let user_id = auth.claims.subject;
    let tenant_id = auth.claims.tenant_id;
    let trace_id = Uuid::new_v4();

    let pending =
        sqlx::query_as::<_, PendingSecret>("SELECT mfa_pending_secret FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|err| {
                error!(user_id = %user_id, error = %err, "Failed to fetch pending MFA secret");
                AuthError::internal_error("Unable to verify MFA enrollment")
            })?;

    let pending_secret = match pending.and_then(|row| row.mfa_pending_secret) {
        Some(secret) => secret,
        None => {
            return Err(AuthError::with_detail(
                StatusCode::BAD_REQUEST,
                "MFA_ENROLLMENT_NOT_STARTED",
                "Start enrollment before verifying MFA.",
                None,
            ))
        }
    };

    let code = normalize_mfa_code(&payload.code).ok_or_else(|| AuthError::mfa_invalid())?;

    if !verify_totp_code(&pending_secret, &code) {
        warn!(
            event = "mfa.enrollment.verify_failed",
            user_id = %user_id,
            tenant_id = %tenant_id,
            trace_id = %trace_id,
            "MFA verification failed"
        );

        let event = MfaActivityEvent {
            action: "mfa.enrollment.verify_failed",
            severity: "warn",
            tenant_id,
            user_id: Some(user_id),
            trace_id,
            occurred_at: Utc::now(),
            ip: None,
            user_agent: None,
            device: None,
            role: None,
            detail: Some(json!({ "reason": "verify_failed" }).to_string()),
        };
        state.emit_mfa_activity(event, None).await;
        return Err(AuthError::mfa_invalid());
    }

    sqlx::query(
        "UPDATE users SET mfa_secret = $1, mfa_pending_secret = NULL, mfa_enrolled_at = NOW(), mfa_failed_attempts = 0 WHERE id = $2",
    )
    .bind(&pending_secret)
    .bind(user_id)
    .execute(&state.db)
    .await
    .map_err(|err| {
        error!(user_id = %user_id, error = %err, "Failed to persist verified MFA secret");
        AuthError::internal_error("Unable to complete MFA enrollment")
    })?;

    info!(
        event = "mfa.enrollment.completed",
        user_id = %user_id,
        tenant_id = %tenant_id,
        trace_id = %trace_id,
        "MFA enrollment verified"
    );

    let event = MfaActivityEvent {
        action: "mfa.enrollment.completed",
        severity: "info",
        tenant_id,
        user_id: Some(user_id),
        trace_id,
        occurred_at: Utc::now(),
        ip: None,
        user_agent: None,
        device: None,
        role: None,
        detail: Some(json!({ "result": "verified" }).to_string()),
    };
    state.emit_mfa_activity(event, None).await;

    Ok(Json(MfaVerifyResponse { enabled: true }))
}
