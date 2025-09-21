use anyhow::Context;
use axum::{
    extract::FromRef,
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method,
    },
    routing::{get, post},
    Router,
};
use common_auth::{JwtConfig, JwtVerifier};
use futures_util::StreamExt;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::Message;
use sqlx::PgPool;
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};
use uuid::Uuid;

mod order_handlers;
use order_handlers::{clear_offline_orders, create_order, list_orders, refund_order, OrderItem};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub kafka_producer: FutureProducer,
    pub pending_orders: Option<Arc<Mutex<HashMap<Uuid, (Vec<OrderItem>, Option<Uuid>, bool)>>>>,
    pub jwt_verifier: Arc<JwtVerifier>,
}

impl FromRef<AppState> for Arc<JwtVerifier> {
    fn from_ref(state: &AppState) -> Self {
        state.jwt_verifier.clone()
    }
}

#[derive(serde::Deserialize, Debug)]
struct PaymentCompletedEvent {
    pub order_id: Uuid,
    pub tenant_id: Uuid,
    pub amount: f64,
}

#[derive(serde::Deserialize, Debug)]
struct PaymentFailedEvent {
    pub order_id: Uuid,
    pub tenant_id: Uuid,
    pub method: String,
    pub reason: String,
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db = PgPool::connect(&database_url).await?;

    let kafka_producer: FutureProducer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .create()
        .expect("failed to create kafka producer");

    let pending_orders: Arc<Mutex<HashMap<Uuid, (Vec<OrderItem>, Option<Uuid>, bool)>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    let state = AppState {
        db: db.clone(),
        kafka_producer: kafka_producer.clone(),
        pending_orders: Some(pending_orders.clone()),
        jwt_verifier,
    };

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
            ]
            .into_iter()
            .collect::<Vec<_>>(),
        );

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/orders", post(create_order).get(list_orders))
        .route("/orders/offline/clear", post(clear_offline_orders))
        .route("/orders/refund", post(refund_order))
        .with_state(state.clone())
        .layer(cors);

    let db_pool = db.clone();
    let producer = kafka_producer.clone();
    let pending_orders_consumer = pending_orders.clone();
    tokio::spawn(async move {
        let consumer: StreamConsumer = rdkafka::ClientConfig::new()
            .set(
                "bootstrap.servers",
                &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
            )
            .set("group.id", "order-service")
            .create()
            .expect("failed to create kafka consumer");
        consumer
            .subscribe(&["payment.completed", "payment.failed"])
            .expect("failed to subscribe");
        let mut stream = consumer.stream();
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(m) => {
                    let topic = m.topic();
                    if let Some(Ok(payload)) = m.payload_view::<str>() {
                        match topic {
                            "payment.completed" => {
                                if let Ok(evt) =
                                    serde_json::from_str::<PaymentCompletedEvent>(payload)
                                {
                                    let _ = sqlx::query(
                                        "UPDATE orders SET status = 'COMPLETED' WHERE id =  AND tenant_id =  AND status <> 'REFUNDED'",
                                    )
                                    .bind(evt.order_id)
                                    .bind(evt.tenant_id)
                                    .execute(&db_pool)
                                    .await;

                                    let maybe_data = {
                                        let mut map = pending_orders_consumer.lock().unwrap();
                                        map.remove(&evt.order_id)
                                    };

                                    if let Some((items, cust_opt, offline)) = maybe_data {
                                        let order_event = serde_json::json!({
                                            "order_id": evt.order_id,
                                            "tenant_id": evt.tenant_id,
                                            "items": items,
                                            "total": evt.amount,
                                            "customer_id": cust_opt,
                                            "offline": offline
                                        });
                                        if let Err(err) = producer
                                            .send(
                                                FutureRecord::to("order.completed")
                                                    .payload(&order_event.to_string())
                                                    .key(&evt.tenant_id.to_string()),
                                                Duration::from_secs(0),
                                            )
                                            .await
                                        {
                                            tracing::error!(
                                                "Failed to send order.completed: {:?}",
                                                err
                                            );
                                        } else {
                                            tracing::info!(
                                                "Order {} marked COMPLETED (payment confirmed)",
                                                evt.order_id
                                            );
                                        }
                                    }
                                } else {
                                    tracing::error!("Failed to parse PaymentCompletedEvent");
                                }
                            }
                            "payment.failed" => {
                                if let Ok(evt) = serde_json::from_str::<PaymentFailedEvent>(payload)
                                {
                                    match sqlx::query(
                                        "UPDATE orders SET status = 'NOT_ACCEPTED' WHERE id =  AND tenant_id =  AND status = 'PENDING'",
                                    )
                                    .bind(evt.order_id)
                                    .bind(evt.tenant_id)
                                    .execute(&db_pool)
                                    .await
                                    {
                                        Ok(result) => {
                                            if result.rows_affected() == 0 {
                                                tracing::warn!(
                                                    order_id = %evt.order_id,
                                                    tenant_id = %evt.tenant_id,
                                                    method = evt.method.as_str(),
                                                    reason = %evt.reason,
                                                    "Payment failure received but order not updated"
                                                );
                                            } else {
                                                tracing::warn!(
                                                    order_id = %evt.order_id,
                                                    tenant_id = %evt.tenant_id,
                                                    method = evt.method.as_str(),
                                                    reason = %evt.reason,
                                                    "Order marked NOT_ACCEPTED due to payment failure"
                                                );
                                            }
                                        }
                                        Err(err) => {
                                            tracing::error!(?err, "Failed to update order for payment failure");
                                        }
                                    }
                                } else {
                                    tracing::error!("Failed to parse PaymentFailedEvent");
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Err(err) => tracing::error!(?err, "Kafka error"),
            }
        }
    });

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8084);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));
    println!("starting order-service on {addr}");
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
