use anyhow::Context;
use axum::{
    body::Body,
    extract::State,
    http::{
        header::{self, ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method, Request, StatusCode,
    },
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Router,
};
// chrono::Utc not directly used in main after state extraction
use common_auth::{JwtConfig, JwtVerifier};
use common_http_errors::{ApiError};
use common_money::log_rounding_mode_once;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::FutureProducer;
use reqwest::Client;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};
// Replaced local HTTP error metrics with shared helper in common-http-errors
use uuid::Uuid;

// Library modules declared in lib.rs; avoid redeclaring to prevent duplicate crate instances.

// use integration_gateway::alerts::{post_alert_webhook, RateLimitAlertEvent}; // not needed in main after state extraction
// Removed direct alert publish import; alerting handled within handlers when feature-enabled.
use integration_gateway::app_state::{AppState, CachedKey};
use integration_gateway::config::GatewayConfig;
use integration_gateway::metrics::GatewayMetrics;
use integration_gateway::rate_limiter::RedisRateLimiter;
use integration_gateway::usage::UsageTracker;

use integration_gateway::integration_handlers::{
    handle_external_order, process_payment, void_payment, ForwardedAuthHeader,
};
// Security context extraction occurs inside handler modules; no direct main.rs usage.
use integration_gateway::webhook_handlers::handle_coinbase_webhook;


async fn health() -> &'static str {
    "ok"
}

async fn metrics_endpoint(State(state): State<AppState>) -> Response {
    match state.metrics.render() {
        Ok(resp) => resp,
        Err(err) => {
            warn!(?err, "Failed to render metrics");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("metrics unavailable"))
                .expect("failed to build metrics error response")
        }
    }
}

use common_http_errors::http_error_metrics_layer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    log_rounding_mode_once();

    let database_url =
        env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration-gateway");
    let db_pool = PgPool::connect(&database_url).await?;

    let config = Arc::new(GatewayConfig::from_env()?);

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

    let rate_limiter = RedisRateLimiter::new(
        &config.redis_url,
        config.rate_limit_window_secs,
        config.redis_prefix.clone(),
    )
    .await
    .expect("Failed to create rate limiter");
    let metrics = Arc::new(GatewayMetrics::new()?);
    // Export configured rate limit target (rpm) for alert comparisons
    metrics.set_rate_limit_rpm_target(config.rate_limit_rpm as i64);
    // Set build info (version / commit) for traceability in metrics
    metrics.set_build_info();
    // Create a small bounded channel representing an internal work queue (placeholder for real queue) and instrument it.
    let (tx, mut rx) = mpsc::channel::<()>(100);
    metrics.set_channel_capacity(100);
    // Spawn a task to drain the channel slowly to simulate work and update depth gauge.
    {
        let metrics_clone = metrics.clone();
        tokio::spawn(async move {
            use tokio::time::sleep;
            use std::time::Duration;
            // Periodically measure depth by peeking (rx capacity not exposed; derive via internal len approximation if available in future)
            // For now we simply set depth to 0 when empty and update on receive events.
            while (rx.recv().await).is_some() {
                // After each item, pretend current depth decreases by 1.
                // In a real implementation we'd capture length via a wrapper.
                metrics_clone.update_channel_depth(0);
                sleep(Duration::from_millis(50)).await;
            }
        });
    }
    let http_client = Client::builder()
        .build()
        .context("Failed to build HTTP client")?;
    let alert_state = Arc::new(Mutex::new(HashMap::new()));

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    // Initialize Kafka producer (feature gated)
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .create()
        .expect("failed to create kafka producer");
    #[cfg(not(any(feature = "kafka", feature = "kafka-producer")))]
    tracing::warn!("kafka features DISABLED (no kafka / kafka-producer): events & alerts will not be published (TA-FND-5)");

    let usage = UsageTracker::new(
        config.clone(),
        db_pool.clone(),
        #[cfg(any(feature = "kafka", feature = "kafka-producer"))] Some(producer.clone())
    );
    usage.spawn_background_tasks();
    let state = AppState {
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] kafka_producer: producer,
        rate_limiter: std::sync::Arc::new(rate_limiter),
        key_cache,
        jwt_verifier,
        metrics: metrics.clone(),
        usage,
        config: config.clone(),
        http_client: http_client.clone(),
        alert_state: alert_state.clone(),
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
        .route("/payments/void", post(void_payment))
        .route("/external/order", post(handle_external_order))
        .route("/webhooks/coinbase", post(handle_coinbase_webhook))
        .layer(middleware::from_fn(move |request, next| {
            let state = auth_state.clone();
            async move { auth_middleware(state, request, next).await }
        }))
        .with_state(protected_state);
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/metrics", get(metrics_endpoint))
        .merge(protected_api)
        .with_state(state)
    .layer(middleware::from_fn(http_error_metrics_adapter)) // existing adapter for ApiError mapping
    .layer(middleware::from_fn(http_error_metrics_layer("integration-gateway")))
        .layer(cors);

    // Best-effort: push a few items to the queue periodically to exercise depth metric (dev visibility only)
    // Guarded so production builds do not emit synthetic backpressure noise (see backlog addendum 2025-10-02 Stabilization Half Items Clarified)
    // Enable only in debug OR when explicitly opted-in via env (non-production troubleshooting / demos)
    if cfg!(debug_assertions) || std::env::var("GATEWAY_DEV_METRICS_DEMO").ok().as_deref() == Some("1") {
        let tx_clone = tx.clone();
        let metrics_clone = metrics.clone();
        tokio::spawn(async move {
            use tokio::time::{sleep, Duration};
            loop {
                for _ in 0..3 { let _ = tx_clone.send(()).await; }
                metrics_clone.update_channel_depth(3);
                sleep(Duration::from_secs(10)).await;
            }
        });
    }

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
) -> Result<Response, ApiError> {
    let headers = request.headers();
    let raw_auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.strip_prefix("Bearer ").map(str::trim));

    let mut claims_opt = None;
    let mut usage_context: Option<(Uuid, String, String)> = None;

    let (limiter_key, tenant_id, identity_label) = if let Some(token) = bearer {
        let claims = state.jwt_verifier.verify(token).map_err(|err| {
            warn!(error = %err, "JWT verification failed");
            ApiError::Forbidden { trace_id: None }
        })?;
        if let Some(header_tid) = headers
            .get("X-Tenant-ID")
            .and_then(|value| value.to_str().ok())
        {
            let header_uuid = Uuid::parse_str(header_tid).map_err(|_| ApiError::BadRequest { code: "invalid_tenant", trace_id: None, message: Some("Invalid tenant header".into()) })?;
            if header_uuid != claims.tenant_id {
                return Err(ApiError::Forbidden { trace_id: None });
            }
        }
        claims_opt = Some(claims.clone());
        (
            format!("jwt:{}:{}", claims.tenant_id, claims.subject),
            claims.tenant_id,
            "jwt",
        )
    } else if let Some(key) = headers.get("X-API-Key").and_then(|h| h.to_str().ok()) {
        let hashed_key = hash_api_key(key);
        let cache = state.key_cache.read().await;
        let cached = cache
            .get(&hashed_key)
            .cloned()
            .ok_or(ApiError::Forbidden { trace_id: None })?;
        drop(cache);
        if let Some(header_tid) = headers
            .get("X-Tenant-ID")
            .and_then(|value| value.to_str().ok())
        {
            let header_uuid = Uuid::parse_str(header_tid).map_err(|_| ApiError::BadRequest { code: "invalid_tenant", trace_id: None, message: Some("Invalid tenant header".into()) })?;
            if header_uuid != cached.tenant_id {
                return Err(ApiError::Forbidden { trace_id: None });
            }
        }
        usage_context = Some((cached.tenant_id, hashed_key.clone(), cached.key_suffix));
        (format!("api:{}", hashed_key), cached.tenant_id, "api")
    } else if let Some(tid_str) = headers.get("X-Tenant-ID").and_then(|h| h.to_str().ok()) {
        let parsed = Uuid::parse_str(tid_str).map_err(|_| ApiError::BadRequest { code: "invalid_tenant", trace_id: None, message: Some("Invalid tenant header".into()) })?;
        (format!("tenant-header:{}", parsed), parsed, "tenant_header")
    } else {
        return Err(ApiError::Forbidden { trace_id: None });
    };

    let rl_start = Instant::now();
    let decision = state
        .rate_limiter
        .check(&limiter_key, state.config.rate_limit_rpm)
        .await
        .map_err(|err| {
            warn!(?err, "Rate limiter failure");
            ApiError::Internal { trace_id: None, message: Some("Rate limiter failure".into()) }
        })?;

    // Record window usage metric and rough latency (TA-PERF-3)
    state.metrics.set_rate_window_usage(decision.current);
    state.metrics.observe_rate_limiter_latency(rl_start.elapsed().as_secs_f64());

    state
        .metrics
        .record_rate_check(identity_label, decision.allowed);

    if let Some((tenant, key_hash, key_suffix)) = usage_context.as_ref() {
        state.record_api_key_metric(decision.allowed);
        state
            .usage
            .record_api_key_use(*tenant, key_hash, key_suffix, decision.allowed)
            .await;
    }

    if !decision.allowed {
        let tenant_opt = usage_context.as_ref().map(|(tenant, _, _)| *tenant);
        let key_hash_opt = usage_context.as_ref().map(|(_, hash, _)| hash.as_str());
        let key_suffix_opt = usage_context.as_ref().map(|(_, _, suffix)| suffix.as_str());
        state
            .maybe_alert_rate_limit(
                identity_label,
                tenant_opt,
                key_hash_opt,
                key_suffix_opt,
                state.config.rate_limit_rpm,
                decision.current,
            )
            .await;
        return Err(ApiError::Forbidden { trace_id: None });
    }

    // Synthesize headers for SecurityCtxExtractor downstream compatibility
    {
        let headers_mut = request.headers_mut();
        headers_mut.insert("X-Tenant-ID", HeaderValue::from_str(&tenant_id.to_string()).unwrap());
        // Assign roles: if JWT claims present and have roles field, map; else default to Support role for now
        if let Some(claims) = &claims_opt {
            // Claims struct role extraction (string vector) assumed via reflection of common_auth
            let role_csv = claims.roles.join(",");
            headers_mut.insert("X-Roles", HeaderValue::from_str(&role_csv).unwrap_or(HeaderValue::from_static("support")));
            headers_mut.insert("X-User-ID", HeaderValue::from_str(&claims.subject.to_string()).unwrap());
        } else {
            headers_mut.insert("X-Roles", HeaderValue::from_static("support"));
            headers_mut.insert("X-User-ID", HeaderValue::from_str(&tenant_id.to_string()).unwrap());
        }
    }
    request.extensions_mut().insert(tenant_id);
    if let Some(header_value) = raw_auth_header.clone() {
        request
            .extensions_mut()
            .insert(ForwardedAuthHeader(header_value));
    }
    if let Some(claims) = claims_opt {
        request.extensions_mut().insert(claims);
    }

    Ok(next.run(request).await)
}

async fn http_error_metrics_adapter(req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    // Adapter now just converts ApiError to Response; metrics captured by shared layer earlier.
    let resp = next.run(req).await;
    Ok(resp)
}

async fn load_active_keys(pool: &PgPool) -> anyhow::Result<HashMap<String, CachedKey>> {
    let records = sqlx::query(
        "SELECT api_key_hash, tenant_id, key_suffix FROM integration_keys WHERE revoked_at IS NULL",
    )
    .fetch_all(pool)
    .await?;

    Ok(records
        .into_iter()
        .map(|row| {
            let hash: String = row.get("api_key_hash");
            let tenant_id: Uuid = row.get("tenant_id");
            let key_suffix: String = row.get("key_suffix");
            (
                hash,
                CachedKey {
                    tenant_id,
                    key_suffix,
                },
            )
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
