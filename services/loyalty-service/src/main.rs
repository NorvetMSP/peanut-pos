use anyhow::Context;
use axum::{
    extract::FromRef,
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method,
    },
    routing::get,
    Router,
};
use common_auth::{ JwtConfig, JwtVerifier };
use common_money::log_rounding_mode_once;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use futures::StreamExt;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::consumer::{Consumer, StreamConsumer};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::{FutureProducer, FutureRecord};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::Message;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use serde::Deserialize;
use sqlx::PgPool;
use std::{
    env,
    net::{SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::net::TcpListener;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use uuid::Uuid;
use once_cell::sync::Lazy;
use prometheus::{Registry, IntCounterVec, Opts, Encoder, TextEncoder};

mod api; // expose library module for tests & reuse
pub use crate::api::{AppState, get_points};

#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
#[derive(Debug, Deserialize)]
struct CompletedEvent {
    order_id: Uuid,
    tenant_id: Uuid,
    total: f64,
    customer_id: Option<Uuid>,
}

impl FromRef<AppState> for Arc<JwtVerifier> {
    fn from_ref(state: &AppState) -> Self { state.jwt_verifier.clone() }
}

#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
async fn handle_completed_event(evt: &CompletedEvent, customer_id: Uuid, pool: &PgPool, producer: &FutureProducer) {
    // Prometheus registry and metrics (module scope)
    // Minimal upsert logic: grant points proportional to total (1 point per whole currency unit)
    let points = evt.total.floor() as i32;
    if points <= 0 { return; }
    if let Err(err) = sqlx::query(
        "INSERT INTO loyalty_points (customer_id, tenant_id, points)
            VALUES ($1,$2,$3)
            ON CONFLICT (customer_id, tenant_id)
            DO UPDATE SET points = loyalty_points.points + EXCLUDED.points",
    )
    .bind(customer_id)
    .bind(evt.tenant_id)
    .bind(points)
    .execute(pool)
    .await
    {
        tracing::warn!(error=%err, "Failed to upsert loyalty points");
        return;
    }
    // Emit a lightweight audit/event (best-effort)
    let event = serde_json::json!({
        "event": "loyalty.points.incremented",
        "tenant_id": evt.tenant_id,
        "customer_id": customer_id,
        "points_added": points,
        "order_id": evt.order_id,
    });
    if let Err(err) = producer.send(
        FutureRecord::to("loyalty.events").payload(&event.to_string()).key(&evt.tenant_id.to_string()),
        Duration::from_secs(0)
    ).await {
        tracing::debug!(error=?err, "Failed to emit loyalty event");
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    log_rounding_mode_once();

    let database_url = env::var("DATABASE_URL")?;
    let db_pool = PgPool::connect(&database_url).await?;

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] let bootstrap = env::var("KAFKA_BOOTSTRAP").unwrap_or_else(|_| "localhost:9092".into());
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &bootstrap)
        .set("group.id", "loyalty-service")
        .create()?;
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] consumer.subscribe(&["order.completed"])?;

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &bootstrap)
        .create()?;

    let state = AppState {
        db: db_pool.clone(),
        jwt_verifier,
        #[cfg(any(feature = "kafka", feature = "kafka-producer"))] producer: producer.clone(),
    };

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] tokio::spawn({
        let db = db_pool.clone();
        let producer = producer.clone();
        async move {
            let mut stream = consumer.stream();
            while let Some(message) = stream.next().await {
                if let Ok(m) = message {
                    if let Some(Ok(text)) = m.payload_view::<str>() {
                        // Inbox de-duplication (env-guarded; default enabled)
                        let inbox_enabled = std::env::var("LOYALTY_INBOX_DEDUP").ok().map(|v| v=="1" || v.eq_ignore_ascii_case("true")).unwrap_or(true);
                        if inbox_enabled {
                            let key_str = m
                                .key()
                                .map(|k| String::from_utf8_lossy(k).to_string())
                                .unwrap_or_else(|| format!("sha1:{}", hex::encode(sha1_smol::Sha1::from(text).digest().bytes())));
                            // Extract tenant_id from payload (stringified UUID)
                            let tenant_hint = serde_json::from_str::<serde_json::Value>(text)
                                .ok()
                                .and_then(|v| v.get("tenant_id").and_then(|t| t.as_str()).map(|s| s.to_string()))
                                .unwrap_or_else(|| "unknown".to_string());
                            let already = sqlx::query_scalar::<_, Option<i64>>(
                                "SELECT 1 FROM inbox WHERE tenant_id = $1 AND message_key = $2 AND topic = $3"
                            )
                            .bind(&tenant_hint)
                            .bind(&key_str)
                            .bind("order.completed")
                            .fetch_optional(&db)
                            .await
                            .ok()
                            .flatten()
                            .is_some();
                            if already { INBOX_DUPLICATES_SKIPPED_TOTAL.with_label_values(&["loyalty-service","order.completed"]).inc(); continue; }
                            let _ = sqlx::query(
                                "INSERT INTO inbox (tenant_id, message_key, topic) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"
                            )
                            .bind(&tenant_hint)
                            .bind(&key_str)
                            .bind("order.completed")
                            .execute(&db)
                            .await;
                            INBOX_INSERTS_TOTAL.with_label_values(&["loyalty-service","order.completed"]).inc();
                        }
                        if let Ok(evt) = serde_json::from_str::<CompletedEvent>(text) {
                            if let Some(cust_id) = evt.customer_id {
                                handle_completed_event(&evt, cust_id, &db, &producer).await;
                            }
                        }
                    }
                }
            }
        }
    });

    let allowed_origins = [
        "http://localhost:3000",
        "http://localhost:3001",
        "http://localhost:5173",
    ];

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(
            allowed_origins
                .iter()
                .filter_map(|origin| origin.parse::<HeaderValue>().ok())
                .collect::<Vec<_>>(),
        ))
        .allow_methods([Method::GET])
        .allow_headers([
            ACCEPT,
            CONTENT_TYPE,
            HeaderName::from_static("authorization"),
            HeaderName::from_static("x-tenant-id"),
        ]);

    use once_cell::sync::Lazy;
    use prometheus::{Registry, IntCounterVec, Opts};
    use axum::middleware;
    static LOYALTY_REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);
    static HTTP_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
        let v = IntCounterVec::new(
            Opts::new("http_errors_total", "Count of HTTP error responses emitted (status >= 400)"),
            &["service", "code", "status"],
        ).unwrap();
        LOYALTY_REGISTRY.register(Box::new(v.clone())).ok();
        v
    });
    static INBOX_INSERTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
        let v = IntCounterVec::new(
            Opts::new("inbox_inserts_total", "Count of inbox insertions for idempotent consumption"),
            &["service","topic"],
        ).unwrap();
        LOYALTY_REGISTRY.register(Box::new(v.clone())).ok();
        v
    });
    static INBOX_DUPLICATES_SKIPPED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
        let v = IntCounterVec::new(
            Opts::new("inbox_duplicates_skipped_total", "Count of duplicate messages skipped due to inbox de-dup"),
            &["service","topic"],
        ).unwrap();
        LOYALTY_REGISTRY.register(Box::new(v.clone())).ok();
        v
    });
    async fn http_error_metrics(req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next) -> axum::response::Response {
        let resp = next.run(req).await;
        let status = resp.status();
        if status.as_u16() >= 400 {
            let code = resp.headers().get("X-Error-Code").and_then(|v| v.to_str().ok()).unwrap_or("unknown");
            HTTP_ERRORS_TOTAL.with_label_values(&["loyalty-service", code, status.as_str()]).inc();
        }
        resp
    }
    async fn metrics() -> (axum::http::StatusCode, String) {
        let encoder = TextEncoder::new();
        let families = LOYALTY_REGISTRY.gather();
        let mut buf = Vec::new();
        if let Err(e) = encoder.encode(&families, &mut buf) {
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("metrics encode error: {e}"));
        }
        (axum::http::StatusCode::OK, String::from_utf8_lossy(&buf).to_string())
    }

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/metrics", get(metrics))
        .route("/points", get(get_points))
        .with_state(state)
        .layer(middleware::from_fn(http_error_metrics))
        .layer(cors);

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8088);
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    info!(%addr, "Starting loyalty-service HTTP server");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(all(test, any(feature = "kafka", feature = "kafka-producer")))]
mod tests {
    use super::*;
    use sqlx::{Executor, PgPool};
    use rdkafka::producer::FutureProducer;

    async fn test_pool() -> PgPool {
        let url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://novapos:novapos@localhost:5432/novapos_test".into());
        let pool = PgPool::connect(&url).await.expect("connect test db");
        // Minimal schema for points table
        pool.execute(
            r#"CREATE TABLE IF NOT EXISTS loyalty_points (
                customer_id UUID NOT NULL,
                tenant_id UUID NOT NULL,
                points INT NOT NULL DEFAULT 0,
                PRIMARY KEY (customer_id, tenant_id)
            )"#,
        )
        .await
        .expect("create table");
        pool
    }

    fn dummy_producer() -> FutureProducer {
        // Use a local bootstrap that may not exist; we won't rely on send success in this test.
        rdkafka::ClientConfig::new()
            .set("bootstrap.servers", "localhost:9092")
            .create()
            .expect("create producer")
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres TEST_DATABASE_URL or local novapos_test)")]
    async fn test_handle_completed_event_upserts_points() {
        let pool = test_pool().await;
        let producer = dummy_producer();
        let customer_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        let evt = CompletedEvent {
            order_id: Uuid::new_v4(),
            tenant_id,
            total: 42.75,
            customer_id: Some(customer_id),
        };
        handle_completed_event(&evt, customer_id, &pool, &producer).await;

        let points: i64 = sqlx::query_scalar(
            "SELECT points FROM loyalty_points WHERE customer_id = $1 AND tenant_id = $2",
        )
        .bind(customer_id)
        .bind(tenant_id)
        .fetch_one(&pool)
        .await
        .expect("fetch points");
        assert!(points > 0, "points should have been incremented");
    }
}

async fn build_jwt_verifier_from_env() -> anyhow::Result<Arc<JwtVerifier>> {
    let issuer = env::var("JWT_ISSUER").context("JWT_ISSUER must be set")?;
    let audience = env::var("JWT_AUDIENCE").context("JWT_AUDIENCE must be set")?;

    let mut config = JwtConfig::new(issuer, audience);
    if let Ok(value) = env::var("JWT_LEEWAY_SECONDS") {
        if let Ok(leeway) = value.parse::<u32>() {
            config = config.with_leeway(leeway);
        }
    }

    let mut builder = JwtVerifier::builder(config);

    if let Ok(url) = env::var("JWT_JWKS_URL") {
        info!(jwks_url = %url, "Configuring JWKS fetcher");
        builder = builder.with_jwks_url(url);
    }

    if let Ok(pem) = env::var("JWT_DEV_PUBLIC_KEY_PEM") {
        warn!("Using JWT_DEV_PUBLIC_KEY_PEM for verification; do not enable in production");
        builder = builder
            .with_rsa_pem("local-dev", pem.as_bytes())
            .map_err(anyhow::Error::from)?;
    }

    let verifier = builder.build().await.map_err(anyhow::Error::from)?;
    info!("JWT verifier initialised");
    Ok(Arc::new(verifier))
}

fn spawn_jwks_refresh(verifier: Arc<JwtVerifier>) {
    let Some(fetcher) = verifier.jwks_fetcher() else {
        return;
    };

    let refresh_secs = env::var("JWKS_REFRESH_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(300);
    let refresh_secs = refresh_secs.max(60);
    let interval_duration = Duration::from_secs(refresh_secs);
    let url = fetcher.url().to_owned();
    let handle = verifier.clone();

    tokio::spawn(async move {
        let mut ticker = interval(interval_duration);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match handle.refresh_jwks().await {
                Ok(count) => {
                    debug!(count, jwks_url = %url, "Refreshed JWKS keys");
                }
                Err(err) => {
                    warn!(error = %err, jwks_url = %url, "Failed to refresh JWKS keys");
                }
            }
        }
    });
}
