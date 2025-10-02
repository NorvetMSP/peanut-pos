use axum::{routing::get, Router};
use product_service::audit_handlers::audit_search;
use product_service::app_state::AppState;
use common_auth::{JwtConfig, JwtVerifier};
use sqlx::PgPool;
use std::{env, sync::Arc};
use tower::util::ServiceExt;
use uuid::Uuid;
use hyper::Request;
use axum::body::Body;
use http_body_util::BodyExt; // for collect
use rdkafka::producer::FutureProducer;

async fn dummy_verifier() -> Arc<JwtVerifier> {
    let cfg = JwtConfig::new(String::from("http://issuer"), String::from("aud"));
    Arc::new(JwtVerifier::new(cfg))
}

#[tokio::test]
async fn prometheus_metrics_exposed() {
    let db_url = match env::var("TEST_AUDIT_DB_URL") { Ok(v) => v, Err(_) => { eprintln!("skipping metrics test: TEST_AUDIT_DB_URL not set"); return; } };
    let pool = PgPool::connect(&db_url).await.expect("connect db");
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
    )"#).execute(&pool).await.unwrap();
    let tenant = Uuid::new_v4();
    sqlx::query("DELETE FROM audit_events WHERE tenant_id = $1").bind(tenant).execute(&pool).await.unwrap();
    let now = chrono::Utc::now();
    sqlx::query("INSERT INTO audit_events (event_id,event_version,tenant_id,actor_id,actor_name,actor_email,entity_type,entity_id,action,severity,source_service,occurred_at,trace_id,payload,meta) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)")
        .bind(Uuid::new_v4())
        .bind(1i32)
        .bind(tenant)
        .bind(Option::<Uuid>::None)
        .bind(Some("Alice".to_string()))
        .bind(Some("alice@example.com".to_string()))
        .bind("order")
        .bind(Option::<Uuid>::None)
        .bind("create")
        .bind("INFO")
        .bind("product-service")
        .bind(now)
        .bind(Option::<Uuid>::None)
        .bind(serde_json::json!({"customer":{"email":"cust@example.com"}}))
        .bind(serde_json::json!({"audit":{"ip":"10.0.0.1"}}))
        .execute(&pool).await.unwrap();
    env::set_var("AUDIT_VIEW_REDACTION_PATHS", "customer.email");
    let kafka: FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers","localhost:9092")
        .set("message.timeout.ms","2000")
        .create()
        .expect("kafka");
    let sink = common_audit::KafkaAuditSink::new(kafka.clone(), common_audit::AuditProducerConfig { topic: "test.audit".into() });
    let base = common_audit::AuditProducer::new(sink);
    let producer = Some(Arc::new(common_audit::BufferedAuditProducer::new(base, 16)));
    let verifier = dummy_verifier().await;
    let state = AppState::new(pool.clone(), kafka, verifier, producer);
    // Use the main crate metrics handler path (product_service::metrics module) via a thin wrapper
    async fn metrics_wrapper(State(app_state): axum::extract::State<AppState>) -> (axum::http::StatusCode, String) {
        // replicate main.rs logic using exported functions
        if let Some(buf) = app_state.audit_buffer() {
            let snap = buf.snapshot();
            product_service::metrics::update_buffer_metrics(snap.queued as u64, snap.emitted as u64, snap.dropped as u64);
        }
        if let Ok(map) = product_service::audit_handlers::VIEW_REDACTIONS_LABELS.lock() {
            use std::collections::HashMap;
            let mut converted: HashMap<(String,String,String), u64> = HashMap::new();
            for ((tenant, role, field), count) in map.iter() { converted.insert((tenant.to_string(), role.clone(), field.clone()), *count); }
            product_service::metrics::update_redaction_counters(product_service::audit_handlers::view_redactions_count() as u64, &converted);
        }
        let out = product_service::metrics::gather(true);
        (axum::http::StatusCode::OK, out)
    }
    use axum::extract::State;
    let app = Router::new()
        .route("/audit/events", get(audit_search))
        .route("/internal/metrics", get(metrics_wrapper))
        .with_state(state);
    // Trigger redaction (Support role, no include_redacted)
    let req = Request::builder()
        .uri(format!("/audit/events?limit=1"))
        .header("X-Tenant-ID", tenant.to_string())
        .header("X-User-ID", Uuid::new_v4().to_string())
        .header("X-Roles", "Support")
        .body(Body::empty()).unwrap();
    app.clone().oneshot(req).await.unwrap();
    // Fetch metrics
    let metrics_req = Request::builder().uri("/internal/metrics").body(Body::empty()).unwrap();
    let resp = app.oneshot(metrics_req).await.unwrap();
    assert!(resp.status().is_success());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("audit_view_redactions_total"));
    assert!(text.contains("audit_buffer_queued"));
}
