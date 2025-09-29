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
use futures::StreamExt;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::Message;
use serde::Deserialize;
use sqlx::{PgPool, Row};
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
) -> Result<String, (axum::http::StatusCode, String)> {
    ensure_role(&auth, LOYALTY_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let cust_id = params
        .get("customer_id")
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or((
            axum::http::StatusCode::BAD_REQUEST,
            "customer_id required".into(),
        ))?;

    let rec =
        sqlx::query("SELECT points FROM loyalty_points WHERE customer_id =  AND tenant_id = ")
            .bind(cust_id)
            .bind(tenant_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("DB error: {}", e),
                )
            })?;

    let points: i32 = rec.try_get("points").map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("DB error: {}", e),
        )
    })?;

    Ok(points.to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let database_url = env::var("DATABASE_URL")?;
    let db_pool = PgPool::connect(&database_url).await?;

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    let bootstrap = env::var("KAFKA_BOOTSTRAP").unwrap_or_else(|_| "localhost:9092".into());
    let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &bootstrap)
        .set("group.id", "loyalty-service")
        .create()?;
    consumer.subscribe(&["order.completed"])?;

    let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &bootstrap)
        .create()?;

    let state = AppState {
        db: db_pool.clone(),
        jwt_verifier,
    };

    tokio::spawn({
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

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/points", get(get_points))
        .with_state(state)
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
        let _ = sqlx::query(
            "INSERT INTO loyalty_points (customer_id, tenant_id, points)
             VALUES (, , )
             ON CONFLICT (customer_id) DO UPDATE
             SET points = loyalty_points.points + EXCLUDED.points",
        )
        .bind(cust_id)
        .bind(evt.tenant_id)
        .bind(delta)
        .execute(db)
        .await;
    }

    if let Ok(record) = sqlx::query("SELECT points FROM loyalty_points WHERE customer_id = ")
        .bind(cust_id)
        .fetch_one(db)
        .await
    {
        if let Ok(new_balance) = record.try_get::<i32, _>("points") {
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
