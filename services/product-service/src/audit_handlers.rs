use axum::{extract::{Query, State}, http::StatusCode, Json};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;
use crate::app_state::AppState;
use common_security::{SecurityCtxExtractor, roles::{ensure_any_role, Role}};
use std::sync::atomic::{AtomicU64, Ordering};
use once_cell::sync::Lazy;
use std::env;
use crate::view_redaction::apply_redaction;

// Global counter for view-layer redactions (TA-AUD-7)
static VIEW_REDACTIONS_TOTAL: AtomicU64 = AtomicU64::new(0);
// Simple in-memory tally map for label breakouts (not thread-safe high perf; acceptable interim) -> Vec<(tenant_id, role, field, count)>
pub static VIEW_REDACTIONS_LABELS: Lazy<std::sync::Mutex<std::collections::HashMap<(uuid::Uuid, String, String), u64>>> = Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

pub fn view_redactions_count() -> u64 { VIEW_REDACTIONS_TOTAL.load(Ordering::Relaxed) }

/// Apply configured redaction paths to payload & meta returning (payload, meta, redacted_field_paths, redaction_count)
pub fn redact_event_fields(mut payload: serde_json::Value, mut meta: serde_json::Value, redaction_paths: &[Vec<String>], include_redacted: bool) -> (serde_json::Value, serde_json::Value, Vec<String>, u64) {
    let mut redacted_fields = Vec::new();
    if redaction_paths.is_empty() { return (payload, meta, redacted_fields, 0); }
    for path in redaction_paths.iter() {
        if path.is_empty() { continue; }
        let mut applied = false;
        if let serde_json::Value::Object(_) = payload {
            if apply_redaction(&mut payload, path, include_redacted) { applied = true; }
        }
        if !applied {
            if let serde_json::Value::Object(_) = meta {
                if apply_redaction(&mut meta, path, include_redacted) { applied = true; }
            }
        }
        if applied { redacted_fields.push(path.join(".")); }
    }
    let count = redacted_fields.len() as u64;
    (payload, meta, redacted_fields, count)
}

fn redaction_paths_from_env() -> &'static Vec<Vec<String>> {
    static PATHS: Lazy<Vec<Vec<String>>> = Lazy::new(|| {
        let raw = env::var("AUDIT_VIEW_REDACTION_PATHS").unwrap_or_else(|_| "".into());
        raw.split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|p| p.split('.').map(|seg| seg.trim().to_string()).collect::<Vec<String>>())
            .collect()
    });
    &PATHS
}

#[derive(Deserialize, Default)]
pub struct AuditQuery {
    pub limit: Option<i64>,
    pub before: Option<String>,
    pub before_event_id: Option<Uuid>,
    pub actor_id: Option<Uuid>,
    pub action: Option<String>,
    pub entity_type: Option<String>,
    pub entity_id: Option<Uuid>,
    pub severity: Option<String>,
    pub trace_id: Option<Uuid>,
    pub include_redacted: Option<bool>, // TA-AUD-7
}

pub async fn audit_search(
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Role enforcement (Admin or Support allowed)
    if let Err(_)= ensure_any_role(&sec, &[Role::Admin, Role::Support]) {
        return Err((StatusCode::FORBIDDEN, "forbidden".into()));
    }

    let mut limit = q.limit.unwrap_or(50);
    if limit < 1 { limit = 1; }
    if limit > 200 { limit = 200; }

    // Build dynamic WHERE clauses
    // We'll construct query using sqlx::query_builder for safety
    use sqlx::QueryBuilder;
    let mut builder: QueryBuilder<sqlx::Postgres> = QueryBuilder::new("SELECT event_id, event_version, tenant_id, actor_id, actor_name, actor_email, entity_type, entity_id, action, severity, source_service, occurred_at, trace_id, payload, meta FROM audit_events WHERE tenant_id = ");
    builder.push_bind(sec.tenant_id);

    if let Some(actor) = q.actor_id { builder.push(" AND actor_id = "); builder.push_bind(actor); }
    if let Some(action) = &q.action { builder.push(" AND action = "); builder.push_bind(action); }
    if let Some(et) = &q.entity_type { builder.push(" AND entity_type = "); builder.push_bind(et); }
    if let Some(entity_id) = q.entity_id { builder.push(" AND entity_id = "); builder.push_bind(entity_id); }
    if let Some(sev) = &q.severity { 
        // normalize severity to TitleCase / uppercase stored variant assumptions
        let norm = sev.to_uppercase();
        builder.push(" AND severity = "); builder.push_bind(norm);
    }
    if let Some(tid) = q.trace_id { builder.push(" AND trace_id = "); builder.push_bind(tid); }
    if let Some(before) = &q.before {
        if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(before) { 
            builder.push(" AND (occurred_at < "); builder.push_bind(ts); builder.push(" OR (occurred_at = "); builder.push_bind(ts); if let Some(eid) = q.before_event_id { builder.push(" AND event_id < "); builder.push_bind(eid); } builder.push(") )");
        }
        else { return Err((StatusCode::BAD_REQUEST, "invalid before timestamp".into())); }
    }

    builder.push(" ORDER BY occurred_at DESC, event_id DESC LIMIT ");
    builder.push_bind(limit);

    let pool: &PgPool = &state.db;
    let rows = builder.build().fetch_all(pool).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut data = Vec::with_capacity(rows.len());
    let include_redacted = q.include_redacted.unwrap_or(false);
    let privileged = sec.roles.iter().any(|r| *r == Role::Admin);

    // Prepare redaction paths once
    let redaction_paths = redaction_paths_from_env();
    let mut total_view_redactions: u64 = 0;
    let mut next_cursor: Option<String> = None;
    let mut next_cursor_event_id: Option<Uuid> = None;
    for row in rows.iter() {
        use sqlx::Row;
        let occurred: chrono::DateTime<chrono::Utc> = row.try_get("occurred_at").unwrap();
    next_cursor = Some(occurred.to_rfc3339());
    next_cursor_event_id = row.try_get::<Uuid,_>("event_id").ok();
        let payload_raw = row.try_get::<serde_json::Value,_>("payload").unwrap_or(serde_json::json!({}));
        let meta_raw = row.try_get::<serde_json::Value,_>("meta").unwrap_or(serde_json::json!({}));
        let (payload, meta, redacted_fields, applied_count) = if !privileged { redact_event_fields(payload_raw, meta_raw, redaction_paths, include_redacted) } else { (payload_raw, meta_raw, Vec::new(), 0) };
        if applied_count > 0 {
            total_view_redactions += applied_count;
            if let Ok(mut guard) = VIEW_REDACTIONS_LABELS.lock() {
                for f in &redacted_fields {
                    let key = (sec.tenant_id, format!("{:?}", sec.roles.first().cloned().unwrap_or(Role::Unknown("none".into()))), f.clone());
                    *guard.entry(key).or_insert(0) += 1;
                }
            }
        }

        data.push(serde_json::json!({
            "event_id": row.try_get::<Uuid,_>("event_id").ok(),
            "event_version": row.try_get::<i32,_>("event_version").ok(),
            "tenant_id": row.try_get::<Uuid,_>("tenant_id").ok(),
            "actor": {
                "id": row.try_get::<Option<Uuid>,_>("actor_id").ok().flatten(),
                "name": row.try_get::<Option<String>,_>("actor_name").ok().flatten(),
                "email": row.try_get::<Option<String>,_>("actor_email").ok().flatten(),
            },
            "entity_type": row.try_get::<String,_>("entity_type").ok(),
            "entity_id": row.try_get::<Option<Uuid>,_>("entity_id").ok().flatten(),
            "action": row.try_get::<String,_>("action").ok(),
            "severity": row.try_get::<String,_>("severity").ok(),
            "source_service": row.try_get::<String,_>("source_service").ok(),
            "occurred_at": occurred.to_rfc3339(),
            "trace_id": row.try_get::<Option<Uuid>,_>("trace_id").ok().flatten(),
            "payload": payload,
            "meta": meta,
            "redacted_fields": redacted_fields,
            "include_redacted": include_redacted,
            "privileged_view": privileged,
        }));
    }
    if total_view_redactions > 0 { VIEW_REDACTIONS_TOTAL.fetch_add(total_view_redactions, Ordering::Relaxed); }

    Ok(Json(serde_json::json!({
        "data": data,
        "next_cursor": next_cursor,
        "next_cursor_event_id": next_cursor_event_id,
        "count": data.len(),
        "limit": limit,
        "view_redactions_applied": total_view_redactions,
    })))
}

// redaction logic moved to view_redaction.rs
