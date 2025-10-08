mod analytics_handlers;

use analytics_handlers::{get_anomalies, get_forecast, get_summary};
use anyhow::Context;
use axum::{
    extract::FromRef,
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method,
    },
    routing::get,
    Router,
};
use common_auth::{JwtConfig, JwtVerifier};
use common_money::log_rounding_mode_once;
use futures_util::StreamExt;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::Message;
use serde_json::Value;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::TcpListener;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Default, Clone, Copy, serde::Serialize)]
pub struct Stats {
    total_sales: f64,
    order_count: u64,
    refund_amount: f64,
    refund_count: u64,
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub data: Arc<Mutex<HashMap<Uuid, Stats>>>,
    pub product_counts: Arc<Mutex<HashMap<Uuid, HashMap<Uuid, i32>>>>,
    pub jwt_verifier: Arc<JwtVerifier>,
}

impl FromRef<AppState> for Arc<JwtVerifier> {
    fn from_ref(state: &AppState) -> Self {
        state.jwt_verifier.clone()
    }
}

#[derive(serde::Serialize)]
struct AnalyticsAlertEvent {
    tenant_id: Uuid,
    alert_type: String,
    details: String,
}

#[derive(serde::Deserialize)]
struct LowStockEvent {
    tenant_id: Uuid,
    product_id: Uuid,
    quantity: i32,
    threshold: i32,
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    log_rounding_mode_once();

    let database_url = env::var("DATABASE_URL")?;
    let db = PgPool::connect(&database_url).await?;

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .set("group.id", "analytics-service")
        .set("enable.auto.commit", "true")
        .create()
        .expect("failed to create kafka consumer");
    consumer.subscribe(&["order.completed", "inventory.low_stock"])?;

    let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .create()
        .expect("failed to create kafka producer");

    let data_map = Arc::new(Mutex::new(HashMap::<Uuid, Stats>::new()));
    let product_counts_map = Arc::new(Mutex::new(HashMap::<Uuid, HashMap<Uuid, i32>>::new()));
    let db_pool = db.clone();
    let data_ref = Arc::clone(&data_map);
    let product_counts_ref = Arc::clone(&product_counts_map);
    let alert_producer = producer.clone();
    tokio::spawn(async move {
        let mut stream = consumer.stream();
        while let Some(message) = stream.next().await {
            if let Ok(m) = message {
                if let Some(Ok(text)) = m.payload_view::<str>() {
                    let topic = m.topic();
                    if topic == "order.completed" {
                        if let Ok(val) = serde_json::from_str::<Value>(text) {
                            if let (Some(tid_str), Some(total_val)) =
                                (val.get("tenant_id"), val.get("total"))
                            {
                                if let (Some(tid), Some(total)) =
                                    (tid_str.as_str(), total_val.as_f64())
                                {
                                    if let Ok(tenant_id) = Uuid::parse_str(tid) {
                                        if let Some(items_val) = val.get("items") {
                                            if let Some(arr) = items_val.as_array() {
                                                let mut counts = product_counts_ref.lock().unwrap();
                                                let tenant_counts = counts
                                                    .entry(tenant_id)
                                                    .or_default();
                                                for item in arr {
                                                    if let Some(pid_str) = item
                                                        .get("product_id")
                                                        .and_then(|v| v.as_str())
                                                    {
                                                        if let Some(qty) = item
                                                            .get("quantity")
                                                            .and_then(|v| v.as_i64())
                                                        {
                                                            if let Ok(pid) =
                                                                Uuid::parse_str(pid_str)
                                                            {
                                                                *tenant_counts
                                                                    .entry(pid)
                                                                    .or_insert(0) += qty as i32;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        let (sales_inc, orders_inc, refunds_inc, refund_count_inc) = {
                                            let mut map = data_ref.lock().unwrap();
                                            let entry = map.entry(tenant_id).or_default();
                                            if total < 0.0 {
                                                entry.refund_count += 1;
                                                entry.refund_amount += -total;
                                                (0.0, 0, -total, 1)
                                            } else {
                                                entry.order_count += 1;
                                                entry.total_sales += total;
                                                (total, 1, 0.0, 0)
                                            }
                                        };
                                        let query = r#"INSERT INTO daily_sales
                                                (tenant_id, date, total_sales, order_count, refund_amount, refund_count)
                                            VALUES (, CURRENT_DATE, , , , )
                                            ON CONFLICT (tenant_id, date)
                                            DO UPDATE
                                               SET total_sales = daily_sales.total_sales + ,
                                                   order_count = daily_sales.order_count + ,
                                                   refund_amount = daily_sales.refund_amount + ,
                                                   refund_count = daily_sales.refund_count + "#;
                                        let _ = sqlx::query(query)
                                            .bind(tenant_id)
                                            .bind(sales_inc)
                                            .bind(orders_inc)
                                            .bind(refunds_inc)
                                            .bind(refund_count_inc)
                                            .execute(&db_pool)
                                            .await;

                                        if refunds_inc > 0.0 {
                                            if let Ok(avg_refund_opt) =
                                                sqlx::query_scalar::<_, Option<f64>>(
                                                    "SELECT AVG(refund_amount) FROM daily_sales                                                  WHERE tenant_id =  AND date < CURRENT_DATE",
                                                )
                                                .bind(tenant_id)
                                                .fetch_one(&db_pool)
                                                .await
                                            {
                                                let avg_refund = avg_refund_opt.unwrap_or(0.0);
                                                if avg_refund > 0.0
                                                    && refunds_inc > 2.0 * avg_refund
                                                {
                                                    let alert = AnalyticsAlertEvent {
                                                        tenant_id,
                                                        alert_type: "HIGH_REFUND_VOLUME".into(),
                                                        details: format!(
                                                            "${:.2} refunded today vs ${:.2} avg",
                                                            refunds_inc, avg_refund
                                                        ),
                                                    };
                                                    let payload =
                                                        serde_json::to_string(&alert).unwrap();
                                                    if let Err(err) = alert_producer
                                                        .send(
                                                            FutureRecord::to("analytics.alert")
                                                                .payload(&payload)
                                                                .key(&tenant_id.to_string()),
                                                            Duration::from_secs(0),
                                                        )
                                                        .await
                                                    {
                                                        tracing::error!(
                                                            "Failed to publish analytics.alert: {:?}",
                                                            err
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if topic == "inventory.low_stock" {
                        if let Ok(evt) = serde_json::from_str::<LowStockEvent>(text) {
                            let alert = AnalyticsAlertEvent {
                                tenant_id: evt.tenant_id,
                                alert_type: "LOW_STOCK".into(),
                                details: format!(
                                    "Product {} down to {} (threshold {})",
                                    evt.product_id, evt.quantity, evt.threshold
                                ),
                            };
                            let payload = serde_json::to_string(&alert).unwrap();
                            let _ = alert_producer
                                .send(
                                    FutureRecord::to("analytics.alert")
                                        .payload(&payload)
                                        .key(&evt.tenant_id.to_string()),
                                    Duration::from_secs(0),
                                )
                                .await;
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

    let app_state = AppState {
        db,
        data: data_map,
        product_counts: product_counts_map,
        jwt_verifier,
    };

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/metrics", get(|| async { (axum::http::StatusCode::OK, String::from("# HELP service_up 1 if the service is running\n# TYPE service_up gauge\nservice_up{service=\"analytics-service\"} 1\n")) }))
        .route("/summary", get(get_summary))
        .route("/forecast", get(get_forecast))
        .route("/anomalies", get(get_anomalies))
        .with_state(app_state)
        .layer(cors);

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8086);
    let addr = SocketAddr::from((host.parse::<std::net::IpAddr>()?, port));
    println!("starting analytics-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;
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
