use axum::{middleware, routing::{get, post}, Router, extract::State, http::{Request, StatusCode}, response::Response};
use tokio::net::TcpListener;
use std::net::SocketAddr;
use std::env;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use std::time::{Duration, Instant};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use rdkafka::producer::FutureProducer;
mod integration_handlers;
mod webhook_handlers;
use integration_handlers::process_payment;
use webhook_handlers::handle_coinbase_webhook;
// ...other handler imports...

// Dummy handler stub for demonstration (replace with real one)
async fn handle_external_order() -> &'static str { "external order" }

/// Per-tenant rate limiting state
struct RateInfo { last_reset: Instant, count: u32 }

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    kafka_producer: FutureProducer,
    tenant_keys: HashMap<String, Uuid>,       // API key -> Tenant UUID
    rate_counters: Arc<Mutex<HashMap<String, RateInfo>>>,
}

async fn health() -> &'static str { "ok" }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    // Load tenant API key configuration from JSON file
    let config_path = std::env::var("INTEGRATION_TENANT_CONFIG")
        .unwrap_or_else(|_| "tenant_config.json".to_string());
    let file = std::fs::File::open(config_path)?;
    let tenant_keys: HashMap<String, Uuid> = serde_json::from_reader(file)?;
    tracing::info!("Loaded tenant API keys for {} tenants", tenant_keys.len());

    // Initialize Kafka producer
    let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &std::env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()))
        .create()
        .expect("failed to create kafka producer");
    let state = AppState {
        kafka_producer: producer,
        tenant_keys,
        rate_counters: Arc::new(Mutex::new(HashMap::new())),
    };

    // Build routes with authentication + rate-limiting middleware
    let protected_api = axum::Router::new()
        .route("/payments", post(process_payment))
        .route("/external/order", post(handle_external_order))
        .route("/webhooks/coinbase", post(handle_coinbase_webhook))
        .layer(middleware::from_fn(auth_middleware));
    let app = axum::Router::new()
        .route("/healthz", get(health))
        .merge(protected_api)
        .with_state(state);

    // Start server (bind host/port from env or defaults)
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8083);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));
    println!("starting integration-gateway on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn auth_middleware<B>(
    State(state): State<AppState>,
    mut request: Request<B>,
    next: axum::middleware::Next<B>,
) -> Result<Response, StatusCode> {
    // Extract API key or internal tenant header
    let headers = request.headers();
    let api_key = headers.get("X-API-Key")
        .and_then(|h| h.to_str().ok());
    let tenant_hdr = headers.get("X-Tenant-ID")
        .and_then(|h| h.to_str().ok());
    let tenant_id = if let Some(key) = api_key {
        // External request with API key
        match state.tenant_keys.get(key) {
            Some(&tid) => tid,
            None => return Err(StatusCode::UNAUTHORIZED), // Unknown API key
        }
    } else if let Some(tid_str) = tenant_hdr {
        // Internal request with tenant ID header (trusted internal call)
        Uuid::parse_str(tid_str).map_err(|_| StatusCode::BAD_REQUEST)?
    } else {
        return Err(StatusCode::UNAUTHORIZED); // No auth provided
    };

    // Simple rate limiting: max 60 requests per minute per key/tenant
    let key_str = api_key.map(|k| k.to_string()).unwrap_or_else(|| tenant_id.to_string());
    {
        let mut counters = state.rate_counters.lock().unwrap();
        let entry = counters.entry(key_str.clone()).or_insert(RateInfo {
            last_reset: Instant::now(),
            count: 0
        });
        // Reset count if window elapsed
        if entry.last_reset.elapsed() >= Duration::from_secs(60) {
            entry.last_reset = Instant::now();
            entry.count = 0;
        }
        entry.count += 1;
        if entry.count > 60 {
            tracing::warn!("Rate limit exceeded for key/tenant: {}", key_str);
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
    }

    // Attach tenant_id to request extensions for handlers to use
    request.extensions_mut().insert(tenant_id);
    // Continue to handler
    Ok(next.run(request).await)
}
