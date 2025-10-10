use std::env;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::{middleware, routing::{get, post}, Router};
use axum::http::{header::{ACCEPT, CONTENT_TYPE}, HeaderName, HeaderValue, Method, StatusCode};
use once_cell::sync::Lazy;
use prometheus::{Encoder, IntCounterVec, IntGaugeVec, Opts, Registry, TextEncoder};
use reqwest::Client;
use sqlx::PgPool;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};

use common_auth::{JwtConfig, JwtVerifier};

use crate::order_handlers::{
    clear_offline_orders, create_order, get_order, get_order_receipt, list_orders, list_returns, compute_order,
    refund_order, void_order, create_order_from_skus,
    list_tax_rate_overrides, upsert_tax_rate_override, get_return_policy, upsert_return_policy, issue_return_override,
};

// --- Error metrics (mirrors product/inventory services) ---
pub static ORDER_REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);
static HTTP_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let v = IntCounterVec::new(
        Opts::new("http_errors_total", "Count of HTTP error responses emitted (status >= 400)"),
        &["service", "code", "status"],
    ).unwrap();
    ORDER_REGISTRY.register(Box::new(v.clone())).ok();
    v
});

// POS telemetry metrics (from POS ingestion)
static POS_PRINT_RETRY_COUNTER: Lazy<IntCounterVec> = Lazy::new(|| {
    let v = IntCounterVec::new(
        Opts::new("pos_print_retry_total", "POS print retry events (queued/success/failed)"),
        &["tenant_id", "store_id", "kind"],
    ).unwrap();
    ORDER_REGISTRY.register(Box::new(v.clone())).ok();
    v
});

static POS_PRINT_GAUGES: Lazy<IntGaugeVec> = Lazy::new(|| {
    let v = IntGaugeVec::new(
        Opts::new("pos_print_gauge", "POS print gauges (queue_depth, last_attempt_ms)"),
        &["tenant_id", "store_id", "name"],
    ).unwrap();
    ORDER_REGISTRY.register(Box::new(v.clone())).ok();
    v
});

pub async fn http_error_metrics(req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next) -> axum::response::Response {
    let resp = next.run(req).await;
    let status = resp.status();
    if status.as_u16() >= 400 {
        let code = resp.headers().get("X-Error-Code").and_then(|v| v.to_str().ok()).unwrap_or("unknown");
        HTTP_ERRORS_TOTAL.with_label_values(&["order-service", code, status.as_str()]).inc();
    }
    resp
}

pub async fn health() -> &'static str { "ok" }

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt_verifier: Arc<JwtVerifier>,
    pub http_client: Client,
    pub inventory_base_url: String,
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    pub kafka_producer: rdkafka::producer::FutureProducer,
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    pub audit_producer: Option<Arc<common_audit::BufferedAuditProducer<common_audit::KafkaAuditSink>>>,
}

impl axum::extract::FromRef<AppState> for Arc<JwtVerifier> {
    fn from_ref(state: &AppState) -> Self { state.jwt_verifier.clone() }
}

pub async fn build_jwt_verifier_from_env() -> anyhow::Result<Arc<JwtVerifier>> {
    let issuer = env::var("JWT_ISSUER").context("JWT_ISSUER must be set")?;
    let audience = env::var("JWT_AUDIENCE").context("JWT_AUDIENCE must be set")?;

    let mut config = JwtConfig::new(issuer, audience);
    if let Ok(value) = env::var("JWT_LEEWAY_SECONDS") {
        if let Ok(leeway) = value.parse::<u32>() { config = config.with_leeway(leeway); }
    }
    let mut builder = JwtVerifier::builder(config);
    if let Ok(url) = env::var("JWT_JWKS_URL") {
        info!(jwks_url = %url, "Configuring JWKS fetcher");
        builder = builder.with_jwks_url(url);
    }
    if let Ok(pem) = env::var("JWT_DEV_PUBLIC_KEY_PEM") {
        warn!("Using JWT_DEV_PUBLIC_KEY_PEM for verification; do not enable in production");
        builder = builder.with_rsa_pem("local-dev", pem.as_bytes()).map_err(anyhow::Error::from)?;
    }
    let verifier = builder.build().await.map_err(anyhow::Error::from)?;
    info!("JWT verifier initialised");
    Ok(Arc::new(verifier))
}

pub fn spawn_jwks_refresh(verifier: Arc<JwtVerifier>) {
    let Some(fetcher) = verifier.jwks_fetcher() else { return; };
    let refresh_secs = env::var("JWKS_REFRESH_SECONDS").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(300).max(60);
    let interval_duration = Duration::from_secs(refresh_secs);
    let url = fetcher.url().to_owned();
    let handle = verifier.clone();
    tokio::spawn(async move {
        let mut ticker = interval(interval_duration);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match handle.refresh_jwks().await {
                Ok(count) => { debug!(count, jwks_url = %url, "Refreshed JWKS keys"); }
                Err(err) => { warn!(error = %err, jwks_url = %url, "Failed to refresh JWKS keys"); }
            }
        }
    });
}

pub fn build_router(state: AppState) -> Router {
    let allowed_origins = [
        "http://localhost:3000",
        "http://localhost:3001",
        "http://localhost:5173",
    ];
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(
            allowed_origins.iter().filter_map(|o| o.parse::<HeaderValue>().ok()).collect::<Vec<_>>(),
        ))
        .allow_methods([
            Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS,
        ])
        .allow_headers([
            ACCEPT, CONTENT_TYPE, HeaderName::from_static("authorization"), HeaderName::from_static("x-tenant-id"),
        ]);

    async fn audit_search() -> (StatusCode, &'static str) { (StatusCode::NOT_IMPLEMENTED, "audit search not implemented") }
    async fn audit_metrics(axum::extract::State(state): axum::extract::State<AppState>) -> axum::Json<serde_json::Value> {
        #[cfg(not(any(feature = "kafka", feature = "kafka-producer")))] let _ = &state;
        #[cfg(any(feature = "kafka", feature = "kafka-producer"))] {
            if let Some(buf) = &state.audit_producer {
                let snap = buf.snapshot();
                return axum::Json(serde_json::json!({"queued": snap.queued, "emitted": snap.emitted, "dropped": snap.dropped}));
            }
        }
        axum::Json(serde_json::json!({"queued":0,"emitted":0,"dropped":0}))
    }
    async fn metrics(axum::extract::State(_state): axum::extract::State<AppState>) -> (StatusCode, String) {
        let encoder = TextEncoder::new();
        let families = ORDER_REGISTRY.gather();
        let mut buf = Vec::new();
        if let Err(e) = encoder.encode(&families, &mut buf) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("metrics encode error: {e}"));
        }
        (StatusCode::OK, String::from_utf8_lossy(&buf).to_string())
    }

    // POS telemetry ingestion
    #[derive(serde::Deserialize)]
    struct PosMetricEntry { name: String, value: i64 }
    #[derive(serde::Deserialize)]
    struct PosTelemetryPayload {
        ts: i64,
        #[serde(default)]
        labels: serde_json::Value,
        #[serde(default)]
        counters: Vec<PosMetricEntry>,
        #[serde(default)]
        gauges: Vec<PosMetricEntry>,
    }
    async fn ingest_pos_telemetry(
        axum::extract::State(_state): axum::extract::State<AppState>,
        headers: axum::http::HeaderMap,
        axum::extract::Json(payload): axum::extract::Json<PosTelemetryPayload>,
    ) -> (StatusCode, &'static str) {
        // Extract tenant_id/store_id labels from request headers or payload.labels
        // Expect X-Tenant-ID header and optional X-Store-ID
        // Fallback to labels.tenant_id/store_id in payload
        let mut tenant_id = headers
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                payload
                    .labels
                    .get("tenant_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string()
            });

        let mut store_id = headers
            .get("x-store-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                payload
                    .labels
                    .get("store_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string()
            });

        // Normalize: trim to avoid accidental whitespace
        tenant_id = tenant_id.trim().to_string();
        store_id = store_id.trim().to_string();

        for c in payload.counters {
            match c.name.as_str() {
                n if n.contains("pos.print.retry.queued") => POS_PRINT_RETRY_COUNTER
                    .with_label_values(&[&tenant_id, &store_id, "queued"]).inc_by(c.value as u64),
                n if n.contains("pos.print.retry.success") => POS_PRINT_RETRY_COUNTER
                    .with_label_values(&[&tenant_id, &store_id, "success"]).inc_by(c.value as u64),
                n if n.contains("pos.print.retry.failed") => POS_PRINT_RETRY_COUNTER
                    .with_label_values(&[&tenant_id, &store_id, "failed"]).inc_by(c.value as u64),
                _ => {}
            }
        }
        for g in payload.gauges {
            if g.name.contains("pos.print.queue_depth") {
                POS_PRINT_GAUGES.with_label_values(&[&tenant_id, &store_id, "queue_depth"]).set(g.value);
            } else if g.name.contains("pos.print.retry.last_attempt") {
                POS_PRINT_GAUGES.with_label_values(&[&tenant_id, &store_id, "last_attempt_ms"]).set(g.value);
            }
        }
        (StatusCode::ACCEPTED, "ok")
    }

    Router::new()
        .route("/healthz", get(health))
        .route("/orders", post(create_order).get(list_orders))
        .route("/orders/sku", post(create_order_from_skus))
        .route("/orders/compute", post(compute_order))
        .route("/orders/:order_id", get(get_order))
    .route("/orders/:order_id/receipt", get(get_order_receipt))
    .route("/orders/:order_id/exchange", post(crate::order_handlers::exchange_order))
        .route("/orders/offline/clear", post(clear_offline_orders))
        .route("/orders/:order_id/void", post(void_order))
        .route("/orders/refund", post(refund_order))
        // Reports
        .route("/reports/settlement", get(crate::order_handlers::get_settlement_report))
        .route("/returns", get(list_returns))
        .route("/admin/tax_rate_overrides", get(list_tax_rate_overrides).post(upsert_tax_rate_override))
    .route("/admin/return_policies", get(get_return_policy).post(upsert_return_policy))
    .route("/admin/overrides/returns", post(issue_return_override))
        .route("/audit/events", get(audit_search))
        .route("/internal/audit_metrics", get(audit_metrics))
    .route("/internal/metrics", get(metrics))
    .route("/metrics", get(metrics))
    .route("/pos/telemetry", post(ingest_pos_telemetry))
        .with_state(state)
        .layer(cors)
        .layer(middleware::from_fn(http_error_metrics))
}
