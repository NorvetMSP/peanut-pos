use anyhow::Context;
use axum::{
    extract::State,
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method, StatusCode,
    },
    routing::{get, post, put},
    Router,
};
use common_auth::{JwtConfig, JwtVerifier};
use common_money::log_rounding_mode_once;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::FutureProducer;
use sqlx::PgPool;
use std::{env, net::SocketAddr, sync::Arc};
use tokio::{
    net::TcpListener,
    time::{interval, Duration, MissedTickBehavior},
};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};

use product_service::product_handlers::{
    create_product, delete_product, list_product_audit, list_products, update_product,
};
use product_service::audit_handlers::{audit_search, view_redactions_count, VIEW_REDACTIONS_LABELS};
mod metrics;
use metrics::{
    update_redaction_counters,
    gather as gather_metrics,
    HTTP_ERRORS_TOTAL,
};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use metrics::update_buffer_metrics;
use axum::middleware;
use axum::body::Body;

async fn error_metrics_mw(
    req: axum::http::Request<Body>,
    next: middleware::Next,
) -> axum::response::Response {
    let resp = next.run(req).await;
    let status = resp.status();
    if status.as_u16() >= 400 {
        let code = resp
            .headers()
            .get("x-error-code")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");
        HTTP_ERRORS_TOTAL
            .with_label_values(&["product-service", code, status.as_str()])
            .inc();
    }
    resp
}
use product_service::app_state::AppState;

// Legacy JSON metrics (will be deprecated once dashboards switch to Prometheus scrape)
async fn audit_metrics(State(state): State<AppState>) -> axum::Json<serde_json::Value> {
    #[cfg(not(any(feature = "kafka", feature = "kafka-producer")))] let _ = &state; // silence unused when features off
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    if let Some(buf) = state.audit_buffer() {
        let snap = buf.snapshot();
        return axum::Json(serde_json::json!({
            "queued": snap.queued,
            "emitted": snap.emitted,
            "dropped": snap.dropped
        }));
    }
    axum::Json(serde_json::json!({"queued":0,"emitted":0,"dropped":0}))
}

async fn metrics(State(state): State<AppState>) -> (StatusCode, String) {
    #[cfg(not(any(feature = "kafka", feature = "kafka-producer")))] let _ = &state;
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    if let Some(buf) = state.audit_buffer() {
        let snap = buf.snapshot();
        update_buffer_metrics(snap.queued as u64, snap.emitted as u64, snap.dropped as u64);
    }
    if let Ok(map) = VIEW_REDACTIONS_LABELS.lock() {
        use std::collections::HashMap;
        let mut converted: HashMap<(String,String,String), u64> = HashMap::new();
        for ((tenant, role, field), count) in map.iter() {
            converted.insert((tenant.to_string(), role.clone(), field.clone()), *count);
        }
    update_redaction_counters(view_redactions_count(), &converted);
    }
    let out = gather_metrics(true);
    (StatusCode::OK, out)
}

// AppState now sourced from library module (app_state.rs)

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    log_rounding_mode_once();
    // Initialize database connection pool
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db = PgPool::connect(&database_url).await?;
    // Ensure database schema is up to date before serving traffic
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.set_ignore_missing(true);
    migrator.run(&db).await?;
    // Initialize Kafka producer for downstream events
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    let kafka_producer: FutureProducer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .create()
        .expect("failed to create kafka producer");

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    // Build application state
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    let audit_topic = env::var("AUDIT_TOPIC").unwrap_or_else(|_| "audit.events".to_string());
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    let base = common_audit::AuditProducer::new(common_audit::KafkaAuditSink::new(kafka_producer.clone(), common_audit::AuditProducerConfig { topic: audit_topic.clone() }));
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    let audit_producer = Some(Arc::new(common_audit::BufferedAuditProducer::new(base, 1024)));
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] tracing::info!(topic = %audit_topic, "Audit producer initialized");
    #[cfg(not(any(feature = "kafka", feature = "kafka-producer")))]
    let audit_producer: Option<Arc<()>> = None;
    #[cfg(not(any(feature = "kafka", feature = "kafka-producer")))]
    let kafka_producer = (); // placeholder when kafka disabled
    let state = AppState::new(db, kafka_producer, jwt_verifier, audit_producer);

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
        .allow_methods(
            [
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::DELETE,
                Method::OPTIONS,
            ]
            .into_iter()
            .collect::<Vec<_>>(),
        )
        .allow_headers(
            [
                ACCEPT,
                CONTENT_TYPE,
                HeaderName::from_static("authorization"),
                HeaderName::from_static("x-tenant-id"),
                HeaderName::from_static("x-user-id"),
                HeaderName::from_static("x-user-name"),
                HeaderName::from_static("x-user-email"),
            ]
            .into_iter()
            .collect::<Vec<_>>(),
        );

    // Build application routes
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/products", post(create_product).get(list_products))
        .route("/products/:id", put(update_product).delete(delete_product))
        .route("/products/:id/audit", get(list_product_audit))
        .route("/audit/events", get(audit_search))
        .route("/internal/audit_metrics", get(audit_metrics))
        .route("/internal/metrics", get(metrics))
        .with_state(state)
        .layer(middleware::from_fn(error_metrics_mw))
        .layer(cors);
    // Start server
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8081);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));
    println!("starting product-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
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
