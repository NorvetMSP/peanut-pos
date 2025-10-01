use common_audit::{extract_actor_from_headers, AuditActor, AuditProducer, NoopAuditSink, AuditSeverity};
use axum::http::HeaderMap;
use uuid::Uuid;
use serde_json::json;

#[test]
fn header_overrides_claims() {
	let subject = Uuid::new_v4();
	let claims = json!({"name":"Claim Name","email":"claim@example.com"});
	let mut headers = HeaderMap::new();
	headers.insert("X-User-Name", "Override Name".parse().unwrap());
	headers.insert("X-User-Email", "override@example.com".parse().unwrap());
	let actor = extract_actor_from_headers(&headers, &claims, subject);
	assert_eq!(actor.name.as_deref(), Some("Override Name"));
	assert_eq!(actor.email.as_deref(), Some("override@example.com"));
	assert_eq!(actor.id, Some(subject));
}

#[test]
fn falls_back_to_claims() {
	let subject = Uuid::new_v4();
	let claims = json!({"name":"Claim Name","email":"claim@example.com"});
	let headers = HeaderMap::new();
	let actor = extract_actor_from_headers(&headers, &claims, subject);
	assert_eq!(actor.name.as_deref(), Some("Claim Name"));
	assert_eq!(actor.email.as_deref(), Some("claim@example.com"));
	assert_eq!(actor.id, Some(subject));
}

#[tokio::test]
async fn build_and_emit_event_noop() {
	let producer = AuditProducer::new(NoopAuditSink);
	let tenant = Uuid::new_v4();
	let actor = AuditActor { id: Some(Uuid::new_v4()), name: Some("Test".into()), email: None };
	let ev = producer.emit(tenant, actor, "product", None, "created", "test-service", AuditSeverity::Info, None, json!({"k":"v"}), json!({})).await.expect("emit");
	assert_eq!(ev.tenant_id, tenant);
	assert_eq!(ev.action, "created");
	assert_eq!(ev.entity_type, "product");
}
