use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::{request::Parts, HeaderMap};
use tracing::Span;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use crate::SecurityError;
use crate::roles::Role;
use common_audit::{AuditActor, extract_actor_from_headers};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityContext {
    pub tenant_id: Uuid,
    pub actor: AuditActor,
    pub roles: Vec<Role>,
    pub trace_id: Option<Uuid>,
}

pub struct SecurityCtxExtractor(pub SecurityContext);

fn tenant_from_headers(headers: &HeaderMap) -> Option<Uuid> {
    headers.get("X-Tenant-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
}

fn roles_from_headers(headers: &HeaderMap) -> Vec<Role> {
    headers.get("X-Roles")
        .and_then(|v| v.to_str().ok())
        .map(|csv| csv.split(',').filter(|s| !s.is_empty()).map(Role::from_str).collect())
        .unwrap_or_else(|| Vec::new())
}

fn trace_id_from_headers(headers: &HeaderMap) -> Option<Uuid> {
    headers.get("X-Trace-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
}

#[async_trait]
impl<S> FromRequestParts<S> for SecurityCtxExtractor where S: Send + Sync {
    type Rejection = (axum::http::StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let headers = &parts.headers;
        let tenant_id = tenant_from_headers(headers).ok_or(SecurityError::MissingTenant)?;

        // Placeholder claims extraction - replace with verified JWT claims
        let claims = serde_json::json!({
            "name": headers.get("X-User-Name").and_then(|v| v.to_str().ok()),
            "email": headers.get("X-User-Email").and_then(|v| v.to_str().ok())
        });
        let subject = headers.get("X-User-ID")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4); // fallback random; in real impl should 401

        let actor = extract_actor_from_headers(headers, &claims, subject);
        let roles = roles_from_headers(headers);
        let trace_id = trace_id_from_headers(headers);

        Span::current().record("tenant_id", &tracing::field::display(tenant_id));

        Ok(SecurityCtxExtractor(SecurityContext { tenant_id, actor, roles, trace_id }))
    }
}
