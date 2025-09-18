use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Router,
};
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
use tokio::time::interval;
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
    };

    // Build routes with authentication + rate-limiting middleware
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
        .with_state(state);

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
    // Extract API key or internal tenant header
    let headers = request.headers();
    let api_key = headers.get("X-API-Key").and_then(|h| h.to_str().ok());
    let tenant_hdr = headers.get("X-Tenant-ID").and_then(|h| h.to_str().ok());

    let (tenant_id, limiter_key) = if let Some(key) = api_key {
        let hashed_key = hash_api_key(key);
        let cache = state.key_cache.read().await;
        let tenant = cache
            .get(&hashed_key)
            .copied()
            .ok_or(StatusCode::UNAUTHORIZED)?;
        (tenant, hashed_key)
    } else if let Some(tid_str) = tenant_hdr {
        let tenant = Uuid::parse_str(tid_str).map_err(|_| StatusCode::BAD_REQUEST)?;
        (tenant, tenant.to_string())
    } else {
        return Err(StatusCode::UNAUTHORIZED); // No auth provided
    };

    // Simple rate limiting: max 60 requests per minute per key/tenant
    {
        let mut counters = state.rate_counters.lock().unwrap();
        let entry = counters.entry(limiter_key.clone()).or_insert(RateInfo {
            last_reset: Instant::now(),
            count: 0,
        });
        // Reset count if window elapsed
        if entry.last_reset.elapsed() >= Duration::from_secs(60) {
            entry.last_reset = Instant::now();
            entry.count = 0;
        }
        entry.count += 1;
        if entry.count > 60 {
            tracing::warn!("Rate limit exceeded for key/tenant: {}", limiter_key);
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
    }

    // Attach tenant_id to request extensions for handlers to use
    request.extensions_mut().insert(tenant_id);
    // Continue to handler
    Ok(next.run(request).await)
}

fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

async fn load_active_keys(pool: &PgPool) -> anyhow::Result<HashMap<String, Uuid>> {
    let rows = sqlx::query(
        "SELECT api_key_hash, tenant_id FROM integration_keys WHERE revoked_at IS NULL",
    )
    .fetch_all(pool)
    .await?;

    let mut map = HashMap::with_capacity(rows.len());
    for row in rows {
        let hash: String = row.get("api_key_hash");
        let tenant_id: Uuid = row.get("tenant_id");
        map.insert(hash, tenant_id);
    }

    Ok(map)
}
