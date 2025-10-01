use crate::context::SecurityContext;
use crate::SecurityError;
use tracing::warn;

// Placeholder Role type; expected to be re-exported from common-auth when available.
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    Admin,
    Manager,
    Support,
    Inventory,
    Unknown(String),
}

impl Role {
    pub fn from_str(s: &str) -> Self {
        match s {
            "admin" | "Admin" => Role::Admin,
            "manager" | "Manager" => Role::Manager,
            "support" | "Support" => Role::Support,
            "inventory" | "Inventory" => Role::Inventory,
            other => Role::Unknown(other.to_string()),
        }
    }
}

pub fn ensure_role(ctx: &SecurityContext, required: Role) -> Result<(), SecurityError> {
    if ctx.roles.iter().any(|r| *r == required) { return Ok(()); }
    warn!(tenant_id = %ctx.tenant_id, ?required, roles = ?ctx.roles, "role_check_failed");
    Err(SecurityError::Forbidden)
}

pub fn ensure_any_role(ctx: &SecurityContext, required: &[Role]) -> Result<(), SecurityError> {
    if ctx.roles.iter().any(|r| required.iter().any(|x| x == r)) { return Ok(()); }
    warn!(tenant_id = %ctx.tenant_id, ?required, roles = ?ctx.roles, "any_role_check_failed");
    Err(SecurityError::Forbidden)
}
