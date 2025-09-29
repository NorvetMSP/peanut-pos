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
use reqwest::Client;
use sqlx::PgPool;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};
use uuid::Uuid;

mod order_handlers;
use order_handlers::{
    clear_offline_orders, create_order, get_order, get_order_receipt, list_orders, list_returns,
    refund_order, void_order,
};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub kafka_producer: FutureProducer,
    pub jwt_verifier: Arc<JwtVerifier>,
    pub http_client: Client,
    pub inventory_base_url: String,
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

#[derive(sqlx::FromRow)]
struct OrderFinancialSummary {
    total: Option<f64>,
    customer_id: Option<Uuid>,
    offline: bool,
    payment_method: String,
}

#[derive(sqlx::FromRow)]
struct OrderItemFinancialRow {
    product_id: Uuid,
    quantity: i32,
    unit_price: f64,
    line_total: f64,
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

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    let http_client = Client::new();
    let inventory_base_url =
        env::var("INVENTORY_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8087".to_string());

    let state = AppState {
        db: db.clone(),
        kafka_producer: kafka_producer.clone(),
        jwt_verifier,
        http_client: http_client.clone(),
        inventory_base_url: inventory_base_url.clone(),
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
        .route("/orders/:order_id", get(get_order))
        .route("/orders/:order_id/receipt", get(get_order_receipt))
        .route("/orders/offline/clear", post(clear_offline_orders))
        .route("/orders/:order_id/void", post(void_order))
        .route("/orders/refund", post(refund_order))
        .route("/returns", get(list_returns))
        .with_state(state.clone())
        .layer(cors);

    let db_pool = db.clone();
    let producer = kafka_producer.clone();

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
                                match serde_json::from_str::<PaymentCompletedEvent>(payload) {
                                    Ok(evt) => {
                                        if let Err(err) = sqlx::query(
                                            "UPDATE orders SET status = 'COMPLETED' WHERE id = $1 AND tenant_id = $2 AND status = 'PENDING'",
                                        )
                                        .bind(evt.order_id)
                                        .bind(evt.tenant_id)
                                        .execute(&db_pool)
                                        .await
                                        {
                                            tracing::error!(
                                                ?err,
                                                order_id = %evt.order_id,
                                                tenant_id = %evt.tenant_id,
                                                "Failed to update order status on payment completion"
                                            );
                                        }

                                        match sqlx::query_as::<_, OrderFinancialSummary>(
                                            "SELECT total::FLOAT8 as total, customer_id, offline, payment_method FROM orders WHERE id = $1 AND tenant_id = $2",
                                        )
                                        .bind(evt.order_id)
                                        .bind(evt.tenant_id)
                                        .fetch_optional(&db_pool)
                                        .await
                                        {
                                            Ok(Some(order_row)) => {
                                                match sqlx::query_as::<_, OrderItemFinancialRow>(
                                                    "SELECT product_id, quantity, unit_price::FLOAT8 as unit_price, line_total::FLOAT8 as line_total FROM order_items WHERE order_id = $1",
                                                )
                                                .bind(evt.order_id)
                                                .fetch_all(&db_pool)
                                                .await
                                                {
                                                    Ok(item_rows) => {
                                                        let event_items: Vec<serde_json::Value> = item_rows
                                                            .into_iter()
                                                            .map(|row| {
                                                                serde_json::json!({
                                                                    "product_id": row.product_id,
                                                                    "quantity": row.quantity,
                                                                    "unit_price": row.unit_price,
                                                                    "line_total": row.line_total,
                                                                })
                                                            })
                                                            .collect();

                                                        let event = serde_json::json!({
                                                            "order_id": evt.order_id,
                                                            "tenant_id": evt.tenant_id,
                                                            "items": event_items,
                                                            "total": order_row.total,
                                                            "customer_id": order_row.customer_id,
                                                            "offline": order_row.offline,
                                                            "payment_method": order_row.payment_method,
                                                        });

                                                        if let Err(err) = producer
                                                            .send(
                                                                FutureRecord::to("order.completed")
                                                                    .payload(&event.to_string())
                                                                    .key(&evt.tenant_id.to_string()),
                                                                Duration::from_secs(0),
                                                            )
                                                            .await
                                                        {
                                                            tracing::error!(
                                                                ?err,
                                                                order_id = %evt.order_id,
                                                                tenant_id = %evt.tenant_id,
                                                                "Failed to publish order.completed after payment confirmation"
                                                            );
                                                        } else {
                                                            tracing::info!(
                                                                order_id = %evt.order_id,
                                                                tenant_id = %evt.tenant_id,
                                                                "Order marked COMPLETED after payment confirmation"
                                                            );
                                                        }
                                                    }
                                                    Err(err) => {
                                                        tracing::error!(
                                                            ?err,
                                                            order_id = %evt.order_id,
                                                            tenant_id = %evt.tenant_id,
                                                            "Failed to load order items for payment completion"
                                                        );
                                                    }
                                                }
                                            }
                                            Ok(None) => {
                                                tracing::warn!(
                                                    order_id = %evt.order_id,
                                                    tenant_id = %evt.tenant_id,
                                                    "Payment completion received for unknown order"
                                                );
                                            }
                                            Err(err) => {
                                                tracing::error!(
                                                    ?err,
                                                    order_id = %evt.order_id,
                                                    tenant_id = %evt.tenant_id,
                                                    "Failed to load order for payment completion"
                                                );
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        tracing::error!(
                                            ?err,
                                            "Failed to parse PaymentCompletedEvent"
                                        );
                                    }
                                }
                            }
                            "payment.failed" => {
                                match serde_json::from_str::<PaymentFailedEvent>(payload) {
                                    Ok(evt) => {
                                        match sqlx::query(
                                            "UPDATE orders SET status = 'NOT_ACCEPTED' WHERE id = $1 AND tenant_id = $2 AND status = 'PENDING'",
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
                                                        "Payment failure received but order already processed"
                                                    );
                                                } else {
                                                    tracing::warn!(
                                                        order_id = %evt.order_id,
                                                        tenant_id = %evt.tenant_id,
                                                        method = evt.method.as_str(),
                                                        reason = %evt.reason,
                                                        "Order marked NOT_ACCEPTED due to payment failure"
                                                    );

                                                    match sqlx::query_as::<_, OrderFinancialSummary>(
                                                        "SELECT total::FLOAT8 as total, customer_id, offline, payment_method FROM orders WHERE id = $1 AND tenant_id = $2",
                                                    )
                                                    .bind(evt.order_id)
                                                    .bind(evt.tenant_id)
                                                    .fetch_optional(&db_pool)
                                                    .await
                                                    {
                                                        Ok(Some(order_row)) => {
                                                            match sqlx::query_as::<_, OrderItemFinancialRow>(
                                                                "SELECT product_id, quantity, unit_price::FLOAT8 as unit_price, line_total::FLOAT8 as line_total FROM order_items WHERE order_id = $1",
                                                            )
                                                            .bind(evt.order_id)
                                                            .fetch_all(&db_pool)
                                                            .await
                                                            {
                                                                Ok(item_rows) => {
                                                                    let event_items: Vec<serde_json::Value> = item_rows
                                                                        .into_iter()
                                                                        .map(|row| {
                                                                            serde_json::json!({
                                                                                "product_id": row.product_id,
                                                                                "quantity": row.quantity,
                                                                                "unit_price": row.unit_price,
                                                                                "line_total": row.line_total,
                                                                            })
                                                                        })
                                                                        .collect();

                                                                    let void_reason = if evt.reason.is_empty() {
                                                                        Some(String::from("payment_failed"))
                                                                    } else {
                                                                        Some(format!("payment_failed: {}", evt.reason))
                                                                    };
                                                                    let void_event = serde_json::json!({
                                                                        "order_id": evt.order_id,
                                                                        "tenant_id": evt.tenant_id,
                                                                        "items": event_items,
                                                                        "total": order_row.total.unwrap_or(0.0),
                                                                        "customer_id": order_row.customer_id,
                                                                        "offline": order_row.offline,
                                                                        "payment_method": order_row.payment_method,
                                                                        "reason": void_reason,
                                                                    });

                                                                    if let Err(err) = producer
                                                                        .send(
                                                                            FutureRecord::to("order.voided")
                                                                                .payload(&void_event.to_string())
                                                                                .key(&evt.tenant_id.to_string()),
                                                                            Duration::from_secs(0),
                                                                        )
                                                                        .await
                                                                    {
                                                                        tracing::error!(
                                                                            ?err,
                                                                            order_id = %evt.order_id,
                                                                            tenant_id = %evt.tenant_id,
                                                                            "Failed to emit order.voided after payment failure"
                                                                        );
                                                                    }
                                                                }
                                                                Err(err) => {
                                                                    tracing::error!(
                                                                        ?err,
                                                                        order_id = %evt.order_id,
                                                                        tenant_id = %evt.tenant_id,
                                                                        "Failed to load order items after payment failure"
                                                                    );
                                                                }
                                                            }
                                                        }
                                                        Ok(None) => {
                                                            tracing::error!(
                                                                order_id = %evt.order_id,
                                                                tenant_id = %evt.tenant_id,
                                                                "Order missing when preparing void event after payment failure"
                                                            );
                                                        }
                                                        Err(err) => {
                                                            tracing::error!(
                                                                ?err,
                                                                order_id = %evt.order_id,
                                                                tenant_id = %evt.tenant_id,
                                                                "Failed to fetch order snapshot after payment failure"
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                            Err(err) => {
                                                tracing::error!(
                                                    ?err,
                                                    order_id = %evt.order_id,
                                                    tenant_id = %evt.tenant_id,
                                                    "Failed to update order for payment failure"
                                                );
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        tracing::error!(?err, "Failed to parse PaymentFailedEvent");
                                    }
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
