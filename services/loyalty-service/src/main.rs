use anyhow::Context;
use axum::{
    extract::{FromRef, Query, State},
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method,
    },
    routing::get,
    Router,
};
use common_auth::{
    ensure_role, tenant_id_from_request, AuthContext, JwtConfig, JwtVerifier, ROLE_ADMIN,
    ROLE_CASHIER, ROLE_MANAGER, ROLE_SUPER_ADMIN,
};
use common_http_errors::ApiError;
use common_money::log_rounding_mode_once;
#[cfg(feature = "kafka")] use futures::StreamExt;
#[cfg(feature = "kafka")] use rdkafka::consumer::{Consumer, StreamConsumer};
#[cfg(feature = "kafka")] use rdkafka::producer::{FutureProducer, FutureRecord};
#[cfg(feature = "kafka")] use rdkafka::Message;
use serde::Deserialize;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    env,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::net::TcpListener;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct CompletedEvent {
    order_id: Uuid,
    tenant_id: Uuid,
    total: f64,
    customer_id: Option<Uuid>,
}

#[derive(Clone)]
struct AppState {
    db: PgPool,
    jwt_verifier: Arc<JwtVerifier>,
    #[cfg(feature = "kafka")] producer: FutureProducer,
}

impl FromRef<AppState> for Arc<JwtVerifier> {
    fn from_ref(state: &AppState) -> Self {
        state.jwt_verifier.clone()
    }
}

const LOYALTY_VIEW_ROLES: &[&str] = &[ROLE_SUPER_ADMIN, ROLE_ADMIN, ROLE_MANAGER, ROLE_CASHIER];

async fn get_points(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> Result<String, ApiError> {
    ensure_role(&auth, LOYALTY_VIEW_ROLES).map_err(|_| ApiError::ForbiddenMissingRole { role: "loyalty_view", trace_id: None })?;
    let tenant_id = tenant_id_from_request(&headers, &auth).map_err(|_| ApiError::BadRequest { code: "missing_tenant", trace_id: None, message: Some("Missing tenant id".into()) })?;

    let cust_id = params
        .get("customer_id")
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or(ApiError::BadRequest { code: "missing_customer_id", trace_id: None, message: Some("customer_id required".into()) })?;

    let rec = sqlx::query!(
        r#"SELECT points FROM loyalty_points WHERE customer_id = $1 AND tenant_id = $2"#,
        cust_id,
        tenant_id
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::Internal { trace_id: None, message: Some(format!("DB error: {}", e)) })?;

    Ok(rec.points.to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    log_rounding_mode_once();

    let database_url = env::var("DATABASE_URL")?;
    let db_pool = PgPool::connect(&database_url).await?;

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    #[cfg(feature = "kafka")] let bootstrap = env::var("KAFKA_BOOTSTRAP").unwrap_or_else(|_| "localhost:9092".into());
    #[cfg(feature = "kafka")] let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &bootstrap)
        .set("group.id", "loyalty-service")
        .create()?;
    #[cfg(feature = "kafka")] consumer.subscribe(&["order.completed"])?;

    #[cfg(feature = "kafka")] let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &bootstrap)
        .create()?;

    let state = AppState {
        db: db_pool.clone(),
        jwt_verifier,
        #[cfg(feature = "kafka")] producer: producer.clone(),
    };

    #[cfg(feature = "kafka")] tokio::spawn({
        let db = db_pool.clone();
        let producer = producer.clone();
        async move {
            let mut stream = consumer.stream();
            while let Some(message) = stream.next().await {
                if let Ok(m) = message {
                    if let Some(Ok(text)) = m.payload_view::<str>() {
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
    static LOYALTY_REGISTRY: Lazy<Registry> = Lazy::new(|| Registry::new());
    static HTTP_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
        let v = IntCounterVec::new(
            Opts::new("http_errors_total", "Count of HTTP error responses emitted (status >= 400)"),
            &["service", "code", "status"],
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

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/points", get(get_points))
        .with_state(state)
        .layer(middleware::from_fn(http_error_metrics))
        .layer(cors);

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8088);
    let ip: IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));
    println!("starting loyalty-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

#[cfg(feature = "kafka")]
async fn handle_completed_event(
    evt: &CompletedEvent,
    cust_id: Uuid,
    db: &PgPool,
    producer: &FutureProducer,
) {
    let delta = if evt.total >= 0.0 {
        evt.total.floor() as i32
    } else {
        -(evt.total.abs().floor() as i32)
    };

    if delta != 0 {
        let _ = sqlx::query!(
            r#"INSERT INTO loyalty_points (customer_id, tenant_id, points)
                VALUES ($1, $2, $3)
                ON CONFLICT (customer_id, tenant_id) DO UPDATE
                SET points = loyalty_points.points + EXCLUDED.points"#,
            cust_id,
            evt.tenant_id,
            delta
        )
        .execute(db)
        .await;
    }

    if let Ok(record) = sqlx::query!(
        r#"SELECT points FROM loyalty_points WHERE customer_id = $1 AND tenant_id = $2"#,
        cust_id,
        evt.tenant_id
    )
    .fetch_one(db)
    .await
    {
        let new_balance = record.points;
        let loyalty_event = serde_json::json!({
            "order_id": evt.order_id,
            "customer_id": cust_id,
            "tenant_id": evt.tenant_id,
            "points_delta": delta,
            "new_balance": new_balance
        });
        let _ = producer
            .send(
                FutureRecord::to("loyalty.updated")
                    .payload(&loyalty_event.to_string())
                    .key(&evt.tenant_id.to_string()),
                Duration::from_secs(0),
            )
            .await;
    }
}

#[cfg(all(test, feature = "kafka"))]
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

        let rec = sqlx::query!(
            r#"SELECT points FROM loyalty_points WHERE customer_id = $1 AND tenant_id = $2"#,
            customer_id,
            tenant_id
        )
        .fetch_one(&pool)
        .await
        .expect("fetch points");
        assert!(rec.points > 0, "points should have been incremented");
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
