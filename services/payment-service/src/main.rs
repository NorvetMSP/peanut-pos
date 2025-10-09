use anyhow::Context;
use axum::{
    http::{
        header::{ACCEPT, CONTENT_TYPE},
    HeaderName, HeaderValue, Method,
    },
    routing::{get, post},
    Router,
};
use common_auth::{JwtConfig, JwtVerifier};
use once_cell::sync::Lazy;
use prometheus::{Registry, IntCounterVec, Opts};
use axum::middleware;
use common_money::log_rounding_mode_once;
use std::{env, net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};

use payment_service::{payment_handlers::{process_card_payment, void_card_payment, create_intent, confirm_intent, capture_intent, void_intent, refund_intent}, AppState};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use common_audit::{KafkaAuditSink, AuditProducer, AuditProducerConfig, BufferedAuditProducer};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::FutureProducer;


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    log_rounding_mode_once();

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] let audit_producer = {
        // Simplified: if KAFKA_BROKERS unset we fallback to None
        if let Ok(brokers) = env::var("KAFKA_BROKERS") {
            let producer: FutureProducer = rdkafka::ClientConfig::new()
                .set("bootstrap.servers", &brokers)
                .create()
                .expect("failed kafka producer");
            let sink = KafkaAuditSink::new(producer, AuditProducerConfig { topic: env::var("AUDIT_TOPIC").unwrap_or_else(|_| "audit.events".into()) });
            Some(Arc::new(BufferedAuditProducer::new(AuditProducer::new(sink), 256)))
        } else { None }
    };
    let state = AppState { jwt_verifier, #[cfg(any(feature = "kafka", feature = "kafka-producer"))] audit_producer };

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
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([
            ACCEPT,
            CONTENT_TYPE,
            HeaderName::from_static("authorization"),
            HeaderName::from_static("x-tenant-id"),
        ]);

    static PAYMENT_REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);
    static HTTP_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
        let v = IntCounterVec::new(
            Opts::new("http_errors_total", "Count of HTTP error responses emitted (status >= 400)"),
            &["service", "code", "status"],
        ).unwrap();
        PAYMENT_REGISTRY.register(Box::new(v.clone())).ok();
        v
    });

    async fn http_error_metrics(req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next) -> axum::response::Response {
        let resp = next.run(req).await;
        let status = resp.status();
        if status.as_u16() >= 400 {
            let code = resp.headers().get("X-Error-Code").and_then(|v| v.to_str().ok()).unwrap_or("unknown");
            HTTP_ERRORS_TOTAL.with_label_values(&["payment-service", code, status.as_str()]).inc();
        }
        resp
    }

    async fn metrics() -> (axum::http::StatusCode, String) {
        // Minimal Prometheus text exposition to standardize endpoint; expand later
        let body = "# HELP service_up 1 if the service is running\n# TYPE service_up gauge\nservice_up{service=\"payment-service\"} 1\n";
        (axum::http::StatusCode::OK, body.to_string())
    }

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/metrics", get(metrics))
        .route("/payments", post(process_card_payment))
        .route("/payments/void", post(void_card_payment))
        // Payment intents MVP (HTTP JSON stubs)
        .route("/payment_intents", post(create_intent))
        .route("/payment_intents/confirm", post(confirm_intent))
        .route("/payment_intents/capture", post(capture_intent))
        .route("/payment_intents/void", post(void_intent))
        .route("/payment_intents/refund", post(refund_intent))
        .with_state(state)
        .layer(middleware::from_fn(http_error_metrics))
        .layer(cors);

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| "8086".to_string())
        .parse()?;
    let addr = SocketAddr::new(host.parse()?, port);
    println!("starting payment-service on {addr}");
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
