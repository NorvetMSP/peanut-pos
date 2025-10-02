//! Integration test for /audit/events redaction behavior.
//! Requires TEST_AUDIT_DB_URL env var pointing to a Postgres database.
use axum::{Router, routing::get};
use product_service::audit_handlers::audit_search;
use product_service::app_state::AppState;
use common_auth::{JwtConfig, JwtVerifier};
use sqlx::PgPool;
use std::{env, sync::Arc};
use tower::util::ServiceExt; // for oneshot
use uuid::Uuid;
use hyper::Request;
use axum::body::Body;
use http_body_util::BodyExt; // for collect

// Minimal stub to build a JwtVerifier (unused by SecurityCtxExtractor but required in AppState)
async fn dummy_verifier() -> Arc<JwtVerifier> {
    let cfg = JwtConfig::new(String::from("http://example/issuer"), String::from("aud"));
    Arc::new(JwtVerifier::new(cfg))
}

#[tokio::test]
async fn audit_events_redaction_flow() {
    let db_url = match env::var("TEST_AUDIT_DB_URL") { Ok(v) => v, Err(_) => { eprintln!("skipping: TEST_AUDIT_DB_URL not set"); return; } };
    let pool = PgPool::connect(&db_url).await.expect("connect db");
    // Create minimal table (idempotent)
    sqlx::query(r#"CREATE TABLE IF NOT EXISTS audit_events (
        event_id UUID PRIMARY KEY,
        event_version INT NOT NULL,
        tenant_id UUID NOT NULL,
        actor_id UUID NULL,
        actor_name TEXT NULL,
        actor_email TEXT NULL,
        entity_type TEXT NOT NULL,
        entity_id UUID NULL,
        action TEXT NOT NULL,
        severity TEXT NOT NULL,
        source_service TEXT NOT NULL,
        occurred_at TIMESTAMPTZ NOT NULL,
        trace_id UUID NULL,
        payload JSONB NOT NULL,
        meta JSONB NOT NULL
    )"#).execute(&pool).await.expect("create table");

    // Clean tenant rows
    let tenant_id = Uuid::new_v4();
    sqlx::query("DELETE FROM audit_events WHERE tenant_id = $1")
        .bind(tenant_id).execute(&pool).await.unwrap();

    // Insert two events with sensitive fields: payload.customer.email, meta.audit.ip
    let now = chrono::Utc::now();
    let insert_sql = r#"INSERT INTO audit_events (
        event_id,event_version,tenant_id,actor_id,actor_name,actor_email,entity_type,entity_id,action,severity,source_service,occurred_at,trace_id,payload,meta)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)"#;
    for i in 0..2u8 {
        sqlx::query(insert_sql)
            .bind(Uuid::new_v4())
            .bind(1i32)
            .bind(tenant_id)
            .bind(Option::<Uuid>::None)
            .bind(Some("Alice".to_string()))
            .bind(Some("alice@example.com".to_string()))
            .bind("order")
            .bind(Option::<Uuid>::None)
            .bind(if i==0 {"create"} else {"update"})
            .bind("INFO")
            .bind("product-service")
            .bind(now - chrono::Duration::seconds(i as i64))
            .bind(Option::<Uuid>::None)
            .bind(serde_json::json!({"customer":{"email":"cust@example.com","id":123}}))
            .bind(serde_json::json!({"audit":{"ip":"10.0.0.1"}}))
            .execute(&pool).await.unwrap();
    }

    // Configure redaction paths via env
    env::set_var("AUDIT_VIEW_REDACTION_PATHS", "customer.email,audit.ip");

    // Build dummy Kafka producer (won't be used in this test) + audit buffer
    use rdkafka::producer::FutureProducer;
    let kafka: FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers","localhost:9092")
        .set("message.timeout.ms","5000")
        .create()
        .expect("future producer");
    let verifier = dummy_verifier().await;
    let kafka_sink = common_audit::KafkaAuditSink::new(kafka.clone(), common_audit::AuditProducerConfig { topic: "test.audit".into() });
    let base = common_audit::AuditProducer::new(kafka_sink);
    let audit_producer = Some(Arc::new(common_audit::BufferedAuditProducer::new(base, 16)));
    let state = AppState::new(pool.clone(), kafka, verifier, audit_producer);

    let app = Router::new().route("/audit/events", get(audit_search)).with_state(state);

    // Non-privileged (no Admin role) removal mode
    let req = Request::builder()
        .uri(format!("/audit/events?limit=5"))
        .header("X-Tenant-ID", tenant_id.to_string())
        .header("X-User-ID", Uuid::new_v4().to_string())
        .header("X-Roles", "Support")
        .body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let first = &json["data"][0];
    // Expect email removed
    assert!(first["payload"]["customer"].get("email").is_none());

    // Non-privileged masking mode include_redacted=true
    let req_mask = Request::builder()
        .uri(format!("/audit/events?limit=5&include_redacted=true"))
        .header("X-Tenant-ID", tenant_id.to_string())
        .header("X-User-ID", Uuid::new_v4().to_string())
        .header("X-Roles", "Support")
        .body(Body::empty()).unwrap();
    let resp_mask = app.clone().oneshot(req_mask).await.unwrap();
    let body_mask = resp_mask.into_body().collect().await.unwrap().to_bytes();
    let json_mask: serde_json::Value = serde_json::from_slice(&body_mask).unwrap();
    let first_mask = &json_mask["data"][0];
    assert_eq!(first_mask["payload"]["customer"]["email"], serde_json::json!("****"));

    // Privileged Admin view should retain raw field
    let req_admin = Request::builder()
        .uri(format!("/audit/events?limit=5"))
        .header("X-Tenant-ID", tenant_id.to_string())
        .header("X-User-ID", Uuid::new_v4().to_string())
        .header("X-Roles", "Admin")
        .body(Body::empty()).unwrap();
    let resp_admin = app.oneshot(req_admin).await.unwrap();
    let body_admin = resp_admin.into_body().collect().await.unwrap().to_bytes();
    let json_admin: serde_json::Value = serde_json::from_slice(&body_admin).unwrap();
    let first_admin = &json_admin["data"][0];
    assert_eq!(first_admin["payload"]["customer"]["email"], serde_json::json!("cust@example.com"));
}
