use anyhow::Context;
use axum::{
    extract::FromRef,
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method,
    },
    routing::{delete, get, post},
    Router,
};
use common_auth::{JwtConfig, JwtVerifier};
use futures::StreamExt;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::producer::FutureProducer;
use rdkafka::Message;
use serde::Deserialize;
use sqlx::PgPool;
use std::{env, net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};
use uuid::Uuid;

mod inventory_handlers;
use inventory_handlers::list_inventory;
mod reservation_handlers;
use reservation_handlers::{create_reservation, release_reservation};

pub(crate) const DEFAULT_THRESHOLD: i32 = 5;

#[derive(Deserialize)]
struct OrderCompletedEvent {
    order_id: Uuid,
    tenant_id: Uuid,
    items: Vec<OrderItem>,
}

#[derive(Deserialize)]
struct OrderVoidedEvent {
    order_id: Uuid,
    tenant_id: Uuid,
}

#[derive(Deserialize)]
struct OrderItem {
    product_id: Uuid,
    quantity: i32,
}

#[derive(Deserialize, Debug)]
struct ProductCreatedEvent {
    product_id: Uuid,
    tenant_id: Uuid,
    initial_quantity: Option<i32>,
    threshold: Option<i32>,
}

#[derive(Deserialize, Debug)]
struct PaymentCompletedEvent {
    order_id: Uuid,
    tenant_id: Uuid,
    amount: f64,
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) db: PgPool,
    pub(crate) jwt_verifier: Arc<JwtVerifier>,
}

impl FromRef<AppState> for Arc<JwtVerifier> {
    fn from_ref(state: &AppState) -> Self {
        state.jwt_verifier.clone()
    }
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db_pool = PgPool::connect(&database_url).await?;

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .set("group.id", "inventory-service")
        .set("enable.auto.commit", "true")
        .create()
        .expect("failed to create kafka consumer");
    consumer.subscribe(&[
        "order.completed",
        "order.voided",
        "payment.completed",
        "product.created",
    ])?;

    let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .create()
        .expect("failed to create kafka producer");

    let state = AppState {
        db: db_pool.clone(),
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
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            ACCEPT,
            CONTENT_TYPE,
            HeaderName::from_static("authorization"),
            HeaderName::from_static("x-tenant-id"),
        ]);

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/inventory", get(list_inventory))
        .route("/inventory/reservations", post(create_reservation))
        .route(
            "/inventory/reservations/:order_id",
            delete(release_reservation),
        )
        .with_state(state.clone())
        .layer(cors);

    let db_for_consumer = db_pool.clone();
    tokio::spawn(async move {
        let mut stream = consumer.stream();
        while let Some(message) = stream.next().await {
            match message {
                Ok(m) => {
                    let topic = m.topic();
                    if let Some(Ok(text)) = m.payload_view::<str>() {
                        if topic == "order.completed" {
                            handle_order_completed(text, &db_for_consumer, &producer).await;
                        } else if topic == "order.voided" {
                            handle_order_voided(text, &db_for_consumer).await;
                        } else if topic == "product.created" {
                            handle_product_created(text, &db_for_consumer).await;
                        } else if topic == "payment.completed" {
                            if let Ok(evt) = serde_json::from_str::<PaymentCompletedEvent>(text) {
                                tracing::debug!(order_id = %evt.order_id, tenant_id = %evt.tenant_id, amount = evt.amount, "Payment completed event received (no-op for inventory)");
                            }
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
        .unwrap_or(8087);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));
    println!("starting inventory-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_order_completed(text: &str, db: &PgPool, producer: &FutureProducer) {
    match serde_json::from_str::<OrderCompletedEvent>(text) {
        Ok(event) => {
            let OrderCompletedEvent {
                order_id,
                tenant_id,
                items,
            } = event;

            let mut tx = match db.begin().await {
                Ok(tx) => tx,
                Err(err) => {
                    tracing::error!(?err, order_id = %order_id, tenant_id = %tenant_id, "Failed to open inventory transaction for completion");
                    return;
                }
            };

            let mut alerts: Vec<(Uuid, i32, i32)> = Vec::new();

            for item in items {
                let product_id = item.product_id;
                let quantity_delta = item.quantity;
                let mut attempts = 0;
                let mut latest: Option<(i32, i32)> = None;

                loop {
                    match sqlx::query!(
                        "UPDATE inventory SET quantity = quantity - $1 WHERE product_id = $2 AND tenant_id = $3 RETURNING quantity, threshold",
                        quantity_delta,
                        product_id,
                        tenant_id
                    )
                    .fetch_optional(&mut *tx)
                    .await
                    {
                        Ok(Some(row)) => {
                            latest = Some((row.quantity, row.threshold));
                            break;
                        }
                        Ok(None) if attempts == 0 => {
                            attempts += 1;
                            if let Err(err) = sqlx::query!(
                                "INSERT INTO inventory (product_id, tenant_id, quantity, threshold) VALUES ($1, $2, $3, $4) ON CONFLICT (product_id, tenant_id) DO NOTHING",
                                product_id,
                                tenant_id,
                                0,
                                DEFAULT_THRESHOLD
                            )
                            .execute(&mut *tx)
                            .await
                            {
                                tracing::error!(
                                    ?err,
                                    product_id = %product_id,
                                    tenant_id = %tenant_id,
                                    "Failed to initialise inventory row before completion"
                                );
                                break;
                            }
                            continue;
                        }
                        Ok(None) => {
                            tracing::warn!(
                                product_id = %product_id,
                                tenant_id = %tenant_id,
                                "Inventory record missing for completed order; skipping adjustment"
                            );
                            break;
                        }
                        Err(err) => {
                            tracing::error!(
                                ?err,
                                product_id = %product_id,
                                tenant_id = %tenant_id,
                                "Failed to update inventory for completed order"
                            );
                            break;
                        }
                    }
                }

                if let Err(err) = sqlx::query!(
                    "DELETE FROM inventory_reservations WHERE order_id = $1 AND tenant_id = $2 AND product_id = $3",
                    order_id,
                    tenant_id,
                    product_id
                )
                .execute(&mut *tx)
                .await
                {
                    tracing::error!(
                        ?err,
                        order_id = %order_id,
                        tenant_id = %tenant_id,
                        product_id = %product_id,
                        "Failed to clear reservation after order completion"
                    );
                }

                if let Some((quantity, threshold)) = latest {
                    if quantity <= threshold {
                        alerts.push((product_id, quantity, threshold));
                    }
                }
            }

            if let Err(err) = tx.commit().await {
                tracing::error!(?err, order_id = %order_id, tenant_id = %tenant_id, "Failed to commit inventory updates for completion");
                return;
            }

            for (product_id, quantity, threshold) in alerts {
                let alert = serde_json::json!({
                    "product_id": product_id,
                    "tenant_id": tenant_id,
                    "quantity": quantity,
                    "threshold": threshold
                });
                if let Err(err) = producer
                    .send(
                        rdkafka::producer::FutureRecord::to("inventory.low_stock")
                            .payload(&alert.to_string())
                            .key(&tenant_id.to_string()),
                        Duration::from_secs(0),
                    )
                    .await
                {
                    tracing::error!(
                        ?err,
                        product_id = %product_id,
                        tenant_id = %tenant_id,
                        "Failed to emit inventory.low_stock after completion"
                    );
                }
            }
        }
        Err(err) => tracing::error!(?err, "Failed to parse OrderCompletedEvent"),
    }
}

async fn handle_order_voided(text: &str, db: &PgPool) {
    match serde_json::from_str::<OrderVoidedEvent>(text) {
        Ok(event) => {
            let OrderVoidedEvent {
                order_id,
                tenant_id,
            } = event;

            let mut tx = match db.begin().await {
                Ok(tx) => tx,
                Err(err) => {
                    tracing::error!(?err, order_id = %order_id, tenant_id = %tenant_id, "Failed to open inventory transaction for void");
                    return;
                }
            };

            let reservations = match sqlx::query!(
                "DELETE FROM inventory_reservations WHERE order_id = $1 AND tenant_id = $2 RETURNING product_id, quantity",
                order_id,
                tenant_id
            )
            .fetch_all(&mut *tx)
            .await
            {
                Ok(rows) => rows,
                Err(err) => {
                    tracing::error!(?err, order_id = %order_id, tenant_id = %tenant_id, "Failed to release reservations for voided order");
                    let _ = tx.rollback().await;
                    return;
                }
            };

            for row in reservations.iter() {
                if row.quantity <= 0 {
                    continue;
                }

                let mut attempts = 0;
                loop {
                    match sqlx::query!(
                        "UPDATE inventory SET quantity = quantity + $1 WHERE product_id = $2 AND tenant_id = $3 RETURNING quantity",
                        row.quantity,
                        row.product_id,
                        tenant_id
                    )
                    .fetch_optional(&mut *tx)
                    .await
                    {
                        Ok(Some(_)) => break,
                        Ok(None) if attempts == 0 => {
                            attempts += 1;
                            if let Err(err) = sqlx::query!(
                                "INSERT INTO inventory (product_id, tenant_id, quantity, threshold) VALUES ($1, $2, $3, $4) ON CONFLICT (product_id, tenant_id) DO NOTHING",
                                row.product_id,
                                tenant_id,
                                row.quantity,
                                DEFAULT_THRESHOLD
                            )
                            .execute(&mut *tx)
                            .await
                            {
                                tracing::error!(
                                    ?err,
                                    order_id = %order_id,
                                    tenant_id = %tenant_id,
                                    product_id = %row.product_id,
                                    "Failed to backfill inventory row during void"
                                );
                                break;
                            }
                            continue;
                        }
                        Ok(None) => {
                            tracing::warn!(
                                order_id = %order_id,
                                tenant_id = %tenant_id,
                                product_id = %row.product_id,
                                "Inventory row still missing after insert attempt"
                            );
                            break;
                        }
                        Err(err) => {
                            tracing::error!(
                                ?err,
                                order_id = %order_id,
                                tenant_id = %tenant_id,
                                product_id = %row.product_id,
                                "Failed to restock inventory for voided order"
                            );
                            break;
                        }
                    }
                }
            }

            if let Err(err) = tx.commit().await {
                tracing::error!(?err, order_id = %order_id, tenant_id = %tenant_id, "Failed to commit inventory release for voided order");
            }
        }
        Err(err) => tracing::error!(?err, "Failed to parse OrderVoidedEvent"),
    }
}

async fn handle_product_created(text: &str, db: &PgPool) {
    match serde_json::from_str::<ProductCreatedEvent>(text) {
        Ok(event) => {
            let initial_quantity = event.initial_quantity.unwrap_or(0);
            let threshold = event.threshold.unwrap_or(DEFAULT_THRESHOLD);
            if let Err(err) = sqlx::query!(
                "INSERT INTO inventory (product_id, tenant_id, quantity, threshold) VALUES ($1, $2, $3, $4) ON CONFLICT (product_id, tenant_id) DO NOTHING",
                event.product_id,
                event.tenant_id,
                initial_quantity,
                threshold
            )
            .execute(db)
            .await
            {
                tracing::error!(
                    product_id = %event.product_id,
                    tenant_id = %event.tenant_id,
                    error = %err,
                    "Failed to seed inventory for product"
                );
            } else {
                tracing::info!(
                    product_id = %event.product_id,
                    tenant_id = %event.tenant_id,
                    quantity = initial_quantity,
                    threshold,
                    "Inventory initialized for product"
                );
            }
        }
        Err(err) => tracing::error!(?err, "Failed to parse ProductCreatedEvent"),
    }
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
