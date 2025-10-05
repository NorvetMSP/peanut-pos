use std::env;
use std::time::Duration;
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use axum::{Router, routing::get, http::StatusCode};
use axum::extract::State;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::Message;
use sqlx::PgPool;
use tracing::{info, warn, error};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    ingested: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
    last_offset: Arc<AtomicU64>,
    last_lag: Arc<AtomicU64>,
    last_ingest_unix_ms: Arc<AtomicU64>,
    retention_deleted: Arc<AtomicU64>,
    retention_last_run_ms: Arc<AtomicU64>,
    retention_last_deleted: Arc<AtomicU64>,
    // Histogram buckets (milliseconds) simple manual counters
    ingest_latency_le_5: Arc<AtomicU64>,
    ingest_latency_le_20: Arc<AtomicU64>,
    ingest_latency_le_100: Arc<AtomicU64>,
    ingest_latency_le_500: Arc<AtomicU64>,
    ingest_latency_le_2000: Arc<AtomicU64>,
    ingest_latency_gt_2000: Arc<AtomicU64>,
}

async fn metrics(State(state): State<AppState>) -> (StatusCode, String) {
    let mut out = String::with_capacity(1024);
    out.push_str("# HELP audit_events_ingested_total Total audit events ingested into read model\n");
    out.push_str("# TYPE audit_events_ingested_total counter\n");
    out.push_str(&format!("audit_events_ingested_total {}\n", state.ingested.load(Ordering::Relaxed)));
    out.push_str("# HELP audit_consumer_lag_last Observed partition lag from last poll\n");
    out.push_str("# TYPE audit_consumer_lag_last gauge\n");
    out.push_str(&format!("audit_consumer_lag_last {}\n", state.last_lag.load(Ordering::Relaxed)));
    out.push_str("# HELP audit_events_failed_total Total audit events that failed to persist\n");
    out.push_str("# TYPE audit_events_failed_total counter\n");
    out.push_str(&format!("audit_events_failed_total {}\n", state.failed.load(Ordering::Relaxed)));
    out.push_str("# HELP audit_consumer_last_ingest_timestamp_ms Unix ms of last successful insert\n");
    out.push_str("# TYPE audit_consumer_last_ingest_timestamp_ms gauge\n");
    out.push_str(&format!("audit_consumer_last_ingest_timestamp_ms {}\n", state.last_ingest_unix_ms.load(Ordering::Relaxed)));
    out.push_str("# HELP audit_events_deleted_total Total audit events deleted by retention\n");
    out.push_str("# TYPE audit_events_deleted_total counter\n");
    out.push_str(&format!("audit_events_deleted_total {}\n", state.retention_deleted.load(Ordering::Relaxed)));
    out.push_str("# HELP audit_retention_last_run_timestamp_ms Unix ms of last retention execution\n");
    out.push_str("# TYPE audit_retention_last_run_timestamp_ms gauge\n");
    out.push_str(&format!("audit_retention_last_run_timestamp_ms {}\n", state.retention_last_run_ms.load(Ordering::Relaxed)));
    out.push_str("# HELP audit_retention_last_run_deleted Events deleted in last retention run\n");
    out.push_str("# TYPE audit_retention_last_run_deleted gauge\n");
    out.push_str(&format!("audit_retention_last_run_deleted {}\n", state.retention_last_deleted.load(Ordering::Relaxed)));
    // Prometheus histogram exposition (cumulative buckets + _count + _sum approximation not tracked, emit count only)
    let b5 = state.ingest_latency_le_5.load(Ordering::Relaxed);
    let b20 = state.ingest_latency_le_20.load(Ordering::Relaxed);
    let b100 = state.ingest_latency_le_100.load(Ordering::Relaxed);
    let b500 = state.ingest_latency_le_500.load(Ordering::Relaxed);
    let b2s = state.ingest_latency_le_2000.load(Ordering::Relaxed);
    let bgt = state.ingest_latency_gt_2000.load(Ordering::Relaxed);
    let count = bgt + b2s; // bgt contains >2000 only; cumulative logic below
    out.push_str("# HELP audit_event_ingest_latency_ms Time from event.occurred_at to consumer insert (ms)\n");
    out.push_str("# TYPE audit_event_ingest_latency_ms histogram\n");
    out.push_str(&format!("audit_event_ingest_latency_ms_bucket{{le=\"5\"}} {}\n", b5));
    out.push_str(&format!("audit_event_ingest_latency_ms_bucket{{le=\"20\"}} {}\n", b20));
    out.push_str(&format!("audit_event_ingest_latency_ms_bucket{{le=\"100\"}} {}\n", b100));
    out.push_str(&format!("audit_event_ingest_latency_ms_bucket{{le=\"500\"}} {}\n", b500));
    out.push_str(&format!("audit_event_ingest_latency_ms_bucket{{le=\"2000\"}} {}\n", b2s));
    out.push_str(&format!("audit_event_ingest_latency_ms_bucket{{le=\"+Inf\"}} {}\n", b2s + bgt));
    out.push_str(&format!("audit_event_ingest_latency_ms_count {}\n", b2s + bgt));
    // _sum omitted for now (could approximate by midpoints later)
    (StatusCode::OK, out)
}

async fn health() -> &'static str { "ok" }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db = PgPool::connect(&database_url).await?;

    // Ensure table exists (migration should have run in at least one service)
    sqlx::query("SELECT 1 FROM audit_events LIMIT 1").execute(&db).await.ok();

    let topic = env::var("AUDIT_TOPIC").unwrap_or_else(|_| "audit.events".to_string());

    let enabled = env::var("AUDIT_CONSUMER_ENABLED").unwrap_or_else(|_| "true".into()) == "true";
    let consumer: Option<StreamConsumer> = if enabled {
        let c: StreamConsumer = rdkafka::ClientConfig::new()
            .set("bootstrap.servers", &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()))
            .set("group.id", env::var("AUDIT_CONSUMER_GROUP").unwrap_or("audit-consumer".into()))
            .set("enable.partition.eof", "false")
            .create()?;
        c.subscribe(&[&topic])?;
        Some(c)
    } else {
        info!("audit consumer disabled via AUDIT_CONSUMER_ENABLED=false");
        None
    }; 

    let state = AppState { 
        db: db.clone(), 
        ingested: Arc::new(AtomicU64::new(0)), 
        failed: Arc::new(AtomicU64::new(0)),
        last_offset: Arc::new(AtomicU64::new(0)), 
        last_lag: Arc::new(AtomicU64::new(0)),
        last_ingest_unix_ms: Arc::new(AtomicU64::new(0)),
        retention_deleted: Arc::new(AtomicU64::new(0)),
        retention_last_run_ms: Arc::new(AtomicU64::new(0)),
        retention_last_deleted: Arc::new(AtomicU64::new(0)),
        ingest_latency_le_5: Arc::new(AtomicU64::new(0)),
        ingest_latency_le_20: Arc::new(AtomicU64::new(0)),
        ingest_latency_le_100: Arc::new(AtomicU64::new(0)),
        ingest_latency_le_500: Arc::new(AtomicU64::new(0)),
        ingest_latency_le_2000: Arc::new(AtomicU64::new(0)),
        ingest_latency_gt_2000: Arc::new(AtomicU64::new(0)),
    };
    let app_state = state.clone();

    // Spawn HTTP server for metrics/health
    let http_state = state.clone();
    tokio::spawn(async move {
        let app = Router::new()
            .route("/healthz", get(health))
            .route("/internal/metrics", get(metrics))
            .with_state(http_state);
        let addr = "0.0.0.0:8090".parse().unwrap();
        info!(%addr, "starting audit-consumer http server");
        axum::serve(tokio::net::TcpListener::bind(addr).await.unwrap(), app).await.unwrap();
    });

    // Consume loop
    let db_pool = db.clone();
    let ingested = app_state.ingested.clone();
    let b5 = app_state.ingest_latency_le_5.clone();
    let b20 = app_state.ingest_latency_le_20.clone();
    let b100 = app_state.ingest_latency_le_100.clone();
    let b500 = app_state.ingest_latency_le_500.clone();
    let b2s = app_state.ingest_latency_le_2000.clone();
    let bgt = app_state.ingest_latency_gt_2000.clone();
    let lag_store = app_state.last_lag.clone();
    let failed_ctr = app_state.failed.clone();
    let last_ingest_ts = app_state.last_ingest_unix_ms.clone();
    let batch_size: usize = env::var("AUDIT_CONSUMER_BATCH_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(1).max(1).min(500);
    if let Some(consumer) = consumer {
        tokio::spawn(async move {
            use futures_util::StreamExt;
            let mut stream = consumer.stream();
            let mut buffer = Vec::with_capacity(batch_size);
            while let Some(message) = stream.next().await {
            match message {
                Ok(m) => {
                    // Real lag: high watermark - current offset (per partition)
                    if let (partition, Some(offset)) = (m.partition(), m.offset().to_raw()) {
                        if let Some(topic_name) = m.topic() {
                            if let Ok((_low, high)) = m.owner().unwrap().fetch_watermarks(topic_name, partition, Duration::from_millis(50)) {
                                let lag = high.saturating_sub(offset + 1);
                                lag_store.store(lag as u64, Ordering::Relaxed);
                            }
                        }
                    }
                    if let Some(Ok(payload)) = m.payload_view::<str>() {
                        if let Ok(evt) = serde_json::from_str::<common_audit::AuditEvent>(payload) {
                            buffer.push(evt);
                        } else { warn!("failed to deserialize audit event"); }
                    }
                    if buffer.len() >= batch_size { // flush
                        let mut to_flush = Vec::new();
                        std::mem::swap(&mut buffer, &mut to_flush);
                        for evt in to_flush.into_iter() {
                            let res = sqlx::query(
                                "INSERT INTO audit_events (
                                    event_id, event_version, tenant_id, actor_id, actor_name, actor_email,
                                    entity_type, entity_id, action, severity, source_service, occurred_at,
                                    trace_id, payload, meta
                                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
                                ON CONFLICT (event_id) DO NOTHING"
                            )
                            .bind(evt.event_id)
                            .bind(evt.event_version as i32)
                            .bind(evt.tenant_id)
                            .bind(evt.actor.id)
                            .bind(evt.actor.name.clone())
                            .bind(evt.actor.email.clone())
                            .bind(evt.entity_type.clone())
                            .bind(evt.entity_id.clone())
                            .bind(evt.action.clone())
                            .bind(format!("{:?}", evt.severity))
                            .bind(evt.source_service.clone())
                            .bind(evt.occurred_at)
                            .bind(evt.trace_id)
                            .bind(serde_json::to_value(&evt.payload).unwrap())
                            .bind(serde_json::to_value(&evt.meta).unwrap())
                            .execute(&db_pool)
                            .await;
                            match res {
                                Ok(_) => {
                                    ingested.fetch_add(1, Ordering::Relaxed);
                                    last_ingest_ts.store(chrono::Utc::now().timestamp_millis() as u64, Ordering::Relaxed);
                                    let now = chrono::Utc::now();
                                    let delta_ms = (now - evt.occurred_at).num_milliseconds().max(0) as u64;
                                    if delta_ms <= 5 { b5.fetch_add(1, Ordering::Relaxed); }
                                    else if delta_ms <= 20 { b20.fetch_add(1, Ordering::Relaxed); }
                                    else if delta_ms <= 100 { b100.fetch_add(1, Ordering::Relaxed); }
                                    else if delta_ms <= 500 { b500.fetch_add(1, Ordering::Relaxed); }
                                    else if delta_ms <= 2000 { b2s.fetch_add(1, Ordering::Relaxed); }
                                    else { bgt.fetch_add(1, Ordering::Relaxed); }
                                }
                                Err(e) => { failed_ctr.fetch_add(1, Ordering::Relaxed); error!(?e, "failed to insert audit event"); }
                            }
                        }
                    }
                }
                    Err(e) => warn!(?e, "kafka consumer error")
                }
            }
        });
    }

    // Retention purge task
    let retention_db = db.clone();
    let retention_deleted_ctr = state.retention_deleted.clone();
    let retention_last_run = state.retention_last_run_ms.clone();
    let retention_last_deleted = state.retention_last_deleted.clone();
    let retention_days: i64 = env::var("AUDIT_RETENTION_DAYS").ok().and_then(|v| v.parse().ok()).unwrap_or(30).max(1).min(365);
    let retention_interval_secs: u64 = env::var("AUDIT_RETENTION_INTERVAL_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(600).max(60);
    let retention_dry_run = env::var("AUDIT_RETENTION_DRY_RUN").ok().map(|v| v == "true").unwrap_or(false);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(retention_interval_secs));
        loop {
            ticker.tick().await;
            let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
            let start = chrono::Utc::now();
            // Count eligible
            let count_res = sqlx::query_scalar::<_, i64>("SELECT count(*)::BIGINT FROM audit_events WHERE occurred_at < $1")
                .bind(cutoff)
                .fetch_one(&retention_db)
                .await;
            match count_res {
                Ok(candidate_count) => {
                    if retention_dry_run {
                        tracing::info!(candidate_count, retention_days, "retention dry run - no deletion performed");
                        retention_last_run.store(chrono::Utc::now().timestamp_millis() as u64, Ordering::Relaxed);
                        retention_last_deleted.store(0, Ordering::Relaxed);
                        continue;
                    }
                    if candidate_count > 0 {
                        let del_res = sqlx::query("DELETE FROM audit_events WHERE occurred_at < $1")
                            .bind(cutoff)
                            .execute(&retention_db)
                            .await;
                        match del_res {
                            Ok(done) => {
                                let deleted = done.rows_affected() as u64;
                                if deleted > 0 { retention_deleted_ctr.fetch_add(deleted, Ordering::Relaxed); }
                                retention_last_deleted.store(deleted, Ordering::Relaxed);
                                retention_last_run.store(chrono::Utc::now().timestamp_millis() as u64, Ordering::Relaxed);
                                tracing::info!(deleted, elapsed_ms = (chrono::Utc::now()-start).num_milliseconds(), "retention purge completed");
                            }
                            Err(e) => {
                                tracing::error!(error=?e, "retention purge failed");
                            }
                        }
                    } else {
                        retention_last_run.store(chrono::Utc::now().timestamp_millis() as u64, Ordering::Relaxed);
                        retention_last_deleted.store(0, Ordering::Relaxed);
                    }
                }
                Err(e) => {
                    tracing::error!(error=?e, "retention count failed");
                }
            }
        }
    });

    // Keep process alive (signal handling simplified)
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
