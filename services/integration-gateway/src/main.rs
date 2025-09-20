use anyhow::Context;
use axum::{
    body::Body,
    http::{
        header::{self, ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method, Request, StatusCode,
    },
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Router,
};
use common_auth::{JwtConfig, JwtVerifier};
use rdkafka::producer::FutureProducer;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};
use uuid::Uuid;

mod events;
mod integration_handlers;
mod webhook_handlers;

use integration_handlers::{handle_external_order, process_payment};
use webhook_handlers::handle_coinbase_webhook;

/// Per-tenant rate limiting state
pub struct RateInfo {
    last_reset: Instant,
    count: u32,
}

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub kafka_producer: FutureProducer,
    pub rate_counters: Arc<Mutex<HashMap<String, RateInfo>>>,
    pub key_cache: Arc<RwLock<HashMap<String, Uuid>>>,
    pub jwt_verifier: Arc<JwtVerifier>,
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let database_url =
        env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration-gateway");
    let db_pool = PgPool::connect(&database_url).await?;

    let initial_keys = load_active_keys(&db_pool).await?;
    tracing::info!(
        count = initial_keys.len(),
        "Loaded integration keys into cache"
    );
    let key_cache = Arc::new(RwLock::new(initial_keys));

    let refresh_secs = env::var("KEY_REFRESH_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60);
    {
        let pool = db_pool.clone();
        let cache = key_cache.clone();
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(refresh_secs.max(10)));
            loop {
                ticker.tick().await;
                match load_active_keys(&pool).await {
                    Ok(latest) => {
                        let count = latest.len();
                        let mut guard = cache.write().await;
                        *guard = latest;
                        tracing::debug!(count, "Refreshed integration key cache");
                    }
                    Err(err) => {
                        tracing::warn!(?err, "Failed to refresh integration key cache");
                    }
                }
            }
        });
    }

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    // Initialize Kafka producer
    let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .create()
        .expect("failed to create kafka producer");
    let state = AppState {
        kafka_producer: producer,
        rate_counters: Arc::new(Mutex::new(HashMap::new())),
        key_cache,
        jwt_verifier,
    };

    // Build routes with authentication + rate-limiting middleware
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
            [Method::GET, Method::POST, Method::OPTIONS]
                .into_iter()
                .collect::<Vec<_>>(),
        )
        .allow_headers(
            [
                ACCEPT,
                CONTENT_TYPE,
                HeaderName::from_static("authorization"),
                HeaderName::from_static("x-tenant-id"),
                HeaderName::from_static("x-api-key"),
            ]
            .into_iter()
            .collect::<Vec<_>>(),
        );
    let protected_state = state.clone();
    let auth_state = state.clone();
    let protected_api = Router::new()
        .route("/payments", post(process_payment))
        .route("/external/order", post(handle_external_order))
        .route("/webhooks/coinbase", post(handle_coinbase_webhook))
        .layer(middleware::from_fn(move |request, next| {
            let state = auth_state.clone();
            async move { auth_middleware(state, request, next).await }
        }))
        .with_state(protected_state);
    let app = Router::new()
        .route("/healthz", get(health))
        .merge(protected_api)
        .with_state(state)
        .layer(cors);

    // Start server (bind host/port from env or defaults)
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8083);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));
    println!("starting integration-gateway on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn auth_middleware(
    state: AppState,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let headers = request.headers();
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.strip_prefix("Bearer ").map(str::trim));

    let limiter_key: String;
    let tenant_id;
    let mut claims_opt = None;

    if let Some(token) = bearer {
        let claims = state.jwt_verifier.verify(token).map_err(|err| {
            warn!(error = %err, "JWT verification failed");
            StatusCode::UNAUTHORIZED
        })?;
        if let Some(header_tid) = headers
            .get("X-Tenant-ID")
            .and_then(|value| value.to_str().ok())
        {
            let header_uuid = Uuid::parse_str(header_tid).map_err(|_| StatusCode::BAD_REQUEST)?;
            if header_uuid != claims.tenant_id {
                return Err(StatusCode::FORBIDDEN);
            }
        }
        limiter_key = format!("jwt:{}:{}", claims.tenant_id, claims.subject);
        tenant_id = claims.tenant_id;
        claims_opt = Some(claims);
    } else if let Some(key) = headers.get("X-API-Key").and_then(|h| h.to_str().ok()) {
        let hashed_key = hash_api_key(key);
        let cache = state.key_cache.read().await;
        let tenant = cache
            .get(&hashed_key)
            .copied()
            .ok_or(StatusCode::UNAUTHORIZED)?;
        if let Some(header_tid) = headers
            .get("X-Tenant-ID")
            .and_then(|value| value.to_str().ok())
        {
            let header_uuid = Uuid::parse_str(header_tid).map_err(|_| StatusCode::BAD_REQUEST)?;
            if header_uuid != tenant {
                return Err(StatusCode::FORBIDDEN);
            }
        }
        limiter_key = format!("api:{}", hashed_key);
        tenant_id = tenant;
    } else if let Some(tid_str) = headers.get("X-Tenant-ID").and_then(|h| h.to_str().ok()) {
        let parsed = Uuid::parse_str(tid_str).map_err(|_| StatusCode::BAD_REQUEST)?;
        limiter_key = format!("tenant-header:{}", parsed);
        tenant_id = parsed;
    } else {
        return Err(StatusCode::UNAUTHORIZED);
    }

    {
        let mut counters = state.rate_counters.lock().unwrap();
        let entry = counters.entry(limiter_key.clone()).or_insert(RateInfo {
            last_reset: Instant::now(),
            count: 0,
        });
        if entry.last_reset.elapsed() >= Duration::from_secs(60) {
            entry.last_reset = Instant::now();
            entry.count = 0;
        }
        entry.count += 1;
        let limit = env::var("GATEWAY_RATE_LIMIT_RPM")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(60);
        if entry.count > limit {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
    }

    request.extensions_mut().insert(tenant_id);
    if let Some(claims) = claims_opt {
        request.extensions_mut().insert(claims);
    }

    Ok(next.run(request).await)
}

async fn load_active_keys(pool: &PgPool) -> anyhow::Result<HashMap<String, Uuid>> {
    let records = sqlx::query(
        "SELECT api_key_hash, tenant_id FROM integration_keys WHERE revoked_at IS NULL",
    )
    .fetch_all(pool)
    .await?;

    Ok(records
        .into_iter()
        .map(|row| {
            let hash: String = row.get("api_key_hash");
            let tenant_id: Uuid = row.get("tenant_id");
            (hash, tenant_id)
        })
        .collect())
}

fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
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
