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
use chrono::Utc;
use common_auth::{JwtConfig, JwtVerifier};
use rdkafka::producer::FutureProducer;
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
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};
use uuid::Uuid;

mod alerts;
mod config;
mod events;
mod integration_handlers;
mod metrics;
mod rate_limiter;
mod usage;
mod webhook_handlers;

use crate::alerts::{post_alert_webhook, publish_rate_limit_alert, RateLimitAlertEvent};
use crate::config::GatewayConfig;
use crate::metrics::GatewayMetrics;
use crate::rate_limiter::RateLimiter;
use crate::usage::UsageTracker;

use integration_handlers::{handle_external_order, process_payment};
use webhook_handlers::handle_coinbase_webhook;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub kafka_producer: FutureProducer,
    pub rate_limiter: RateLimiter,
    pub key_cache: Arc<RwLock<HashMap<String, CachedKey>>>,
    pub jwt_verifier: Arc<JwtVerifier>,
    pub metrics: Arc<GatewayMetrics>,
    pub usage: UsageTracker,
    pub config: Arc<GatewayConfig>,
    pub http_client: Client,
    pub alert_state: Arc<Mutex<HashMap<String, Instant>>>,
}

#[derive(Clone)]
pub struct CachedKey {
    pub tenant_id: Uuid,
    pub key_suffix: String,
}

impl AppState {
    pub fn record_api_key_metric(&self, allowed: bool) {
        let result = if allowed { "allowed" } else { "rejected" };
        self.metrics.record_api_key_request(result);
    }

    pub async fn maybe_alert_rate_limit(
        &self,
        identity: &str,
        tenant_id: Option<Uuid>,
        key_hash: Option<&str>,
        key_suffix: Option<&str>,
        limit: u32,
        current: i64,
    ) {
        let threshold = (limit as f64 * self.config.rate_limit_burst_multiplier).ceil() as i64;
        if current < threshold {
            return;
        }

        let alert_key = key_hash
            .map(|hash| format!("api:{}", hash))
            .unwrap_or_else(|| format!("identity:{}", identity));

        {
            let mut guard = self.alert_state.lock().unwrap();
            let now = Instant::now();
            if let Some(last) = guard.get(&alert_key) {
                if now.duration_since(*last).as_secs() < self.config.rate_limit_alert_cooldown_secs
                {
                    return;
                }
            }
            guard.insert(alert_key.clone(), now);
        }

        let suffix_display = key_suffix.unwrap_or("-");
        let message = format!(
            "Rate limit burst detected identity={} tenant={:?} key_suffix={} count={} limit={} window={}s",
            identity,
            tenant_id,
            suffix_display,
            current,
            limit,
            self.config.rate_limit_window_secs,
        );

        warn!(
            ?tenant_id,
            key_hash,
            key_suffix,
            identity,
            current,
            limit,
            window = self.config.rate_limit_window_secs,
            message,
            "Rate limit burst detected"
        );

        let event = RateLimitAlertEvent {
            action: "gateway.rate_limit.alert",
            tenant_id,
            key_hash: key_hash.map(|value| value.to_string()),
            key_suffix: key_suffix.map(|value| value.to_string()),
            identity: identity.to_string(),
            limit,
            count: current,
            window_seconds: self.config.rate_limit_window_secs,
            occurred_at: Utc::now(),
            message: message.clone(),
        };

        if let Err(err) =
            publish_rate_limit_alert(&self.kafka_producer, &self.config.alert_topic, &event).await
        {
            warn!(?err, "Failed to publish rate limit alert");
        }

        if let Some(url) = &self.config.security_alert_webhook_url {
            if let Err(err) = post_alert_webhook(
                &self.http_client,
                url,
                self.config.security_alert_webhook_bearer.as_deref(),
                &message,
            )
            .await
            {
                warn!(?err, "Failed to post security alert webhook");
            }
        }
    }
}

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

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

    let rate_limiter = RateLimiter::new(
        &config.redis_url,
        config.rate_limit_window_secs,
        config.redis_prefix.clone(),
    )
    .await?;

    let metrics = Arc::new(GatewayMetrics::new()?);
    let http_client = Client::builder()
        .build()
        .context("Failed to build HTTP client")?;
    let alert_state = Arc::new(Mutex::new(HashMap::new()));

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

    let usage = UsageTracker::new(config.clone(), db_pool.clone(), producer.clone());
    usage.spawn_background_tasks();
    let state = AppState {
        kafka_producer: producer,
        rate_limiter,
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

    let mut claims_opt = None;
    let mut usage_context: Option<(Uuid, String, String)> = None;

    let (limiter_key, tenant_id, identity_label) = if let Some(token) = bearer {
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
            .ok_or(StatusCode::UNAUTHORIZED)?;
        drop(cache);
        if let Some(header_tid) = headers
            .get("X-Tenant-ID")
            .and_then(|value| value.to_str().ok())
        {
            let header_uuid = Uuid::parse_str(header_tid).map_err(|_| StatusCode::BAD_REQUEST)?;
            if header_uuid != cached.tenant_id {
                return Err(StatusCode::FORBIDDEN);
            }
        }
        usage_context = Some((cached.tenant_id, hashed_key.clone(), cached.key_suffix));
        (format!("api:{}", hashed_key), cached.tenant_id, "api")
    } else if let Some(tid_str) = headers.get("X-Tenant-ID").and_then(|h| h.to_str().ok()) {
        let parsed = Uuid::parse_str(tid_str).map_err(|_| StatusCode::BAD_REQUEST)?;
        (format!("tenant-header:{}", parsed), parsed, "tenant_header")
    } else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let decision = state
        .rate_limiter
        .check(&limiter_key, state.config.rate_limit_rpm)
        .await
        .map_err(|err| {
            warn!(?err, "Rate limiter failure");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

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
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    request.extensions_mut().insert(tenant_id);
    if let Some(claims) = claims_opt {
        request.extensions_mut().insert(claims);
    }

    Ok(next.run(request).await)
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
