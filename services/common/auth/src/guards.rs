use axum::http::{HeaderMap, StatusCode};
use uuid::Uuid;

use crate::AuthContext;

const TENANT_HEADER: &str = "X-Tenant-ID";

#[derive(Debug, Clone)]
pub enum GuardError {
    MissingTenantHeader,
    InvalidTenantHeader,
    TenantMismatch { expected: Uuid, received: Uuid },
    Forbidden { required: Vec<String> },
}

impl GuardError {
    pub fn into_response(self) -> (StatusCode, String) {
        match self {
            GuardError::MissingTenantHeader => (
                StatusCode::BAD_REQUEST,
                format!("Missing {TENANT_HEADER} header"),
            ),
            GuardError::InvalidTenantHeader => (
                StatusCode::BAD_REQUEST,
                format!("Invalid {TENANT_HEADER} header"),
            ),
            GuardError::TenantMismatch { expected, received } => (
                StatusCode::FORBIDDEN,
                format!(
                    "Authenticated tenant ({expected}) does not match {TENANT_HEADER} header ({received})",
                ),
            ),
            GuardError::Forbidden { required } => (
                StatusCode::FORBIDDEN,
                if required.is_empty() {
                    "Insufficient role".to_string()
                } else {
                    format!(
                        "Insufficient role. Required one of: {}",
                        required.join(", ")
                    )
                },
            ),
        }
    }
}

impl From<GuardError> for (StatusCode, String) {
    fn from(value: GuardError) -> Self {
        value.into_response()
    }
}

pub fn ensure_role(auth: &AuthContext, allowed: &[&str]) -> Result<(), GuardError> {
    if allowed.is_empty() {
        return Ok(());
    }

    let has_role = auth
        .claims
        .roles
        .iter()
        .any(|role| allowed.iter().any(|required| role == required));

    if has_role {
        Ok(())
    } else {
        Err(GuardError::Forbidden {
            required: allowed.iter().map(|value| value.to_string()).collect(),
        })
    }
}

pub fn tenant_id_from_request(headers: &HeaderMap, auth: &AuthContext) -> Result<Uuid, GuardError> {
    let claims_tenant = auth.claims.tenant_id;

    match headers.get(TENANT_HEADER) {
        Some(raw) => {
            let value = raw
                .to_str()
                .map_err(|_| GuardError::InvalidTenantHeader)?
                .trim();
            if value.is_empty() {
                return Err(GuardError::InvalidTenantHeader);
            }

            let requested = Uuid::parse_str(value).map_err(|_| GuardError::InvalidTenantHeader)?;
            if requested != claims_tenant {
                return Err(GuardError::TenantMismatch {
                    expected: claims_tenant,
                    received: requested,
                });
            }

            Ok(requested)
        }
        None => Err(GuardError::MissingTenantHeader),
    }
}
