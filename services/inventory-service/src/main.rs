use anyhow::Context;
use axum::{
    extract::{FromRef, State},
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method,
    },
    routing::{delete, get, post},
    Router,
    middleware,
    body::Body,
};
use common_auth::{JwtConfig, JwtVerifier};
use common_money::log_rounding_mode_once;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use futures::StreamExt; // only needed when kafka/kafka-producer feature enabled
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::consumer::{Consumer, StreamConsumer};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::FutureProducer;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::Message;
use sqlx::{PgPool, Row};
use prometheus::{Encoder, TextEncoder};
use common_observability::InventoryMetrics;
use std::{env, net::SocketAddr, sync::Arc, time::Duration};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use serde::Deserialize; // needed for event struct derives when kafka/kafka-producer feature enabled
use tokio::net::TcpListener;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};
use inventory_service::DEFAULT_THRESHOLD; // import shared constant
use uuid::Uuid;

mod inventory_handlers;
use inventory_handlers::list_inventory;
mod reservation_handlers;
use reservation_handlers::{create_reservation, release_reservation};
mod location_handlers;
use location_handlers::list_locations;

// (Removed placeholder error metrics layer; will reintroduce with proper implementation later)

pub(crate) const DEFAULT_RESERVATION_TTL_SECS: i64 = 900; // 15 minutes

#[derive(Deserialize)]
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] struct OrderCompletedEvent {
    order_id: Uuid,
    tenant_id: Uuid,
    items: Vec<OrderItem>,
}

#[derive(Deserialize)]
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] struct OrderVoidedEvent {
    order_id: Uuid,
    tenant_id: Uuid,
}

#[derive(Deserialize)]
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] struct OrderItem {
    product_id: Uuid,
    quantity: i32,
}

#[derive(Deserialize, Debug)]
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] struct ProductCreatedEvent {
    product_id: Uuid,
    tenant_id: Uuid,
    initial_quantity: Option<i32>,
    threshold: Option<i32>,
}

#[derive(Deserialize, Debug)]
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] struct PaymentCompletedEvent {
    order_id: Uuid,
    tenant_id: Uuid,
    amount: f64,
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt_verifier: Arc<JwtVerifier>,
    #[allow(dead_code)]
    pub multi_location_enabled: bool,
    #[allow(dead_code)]
    pub reservation_default_ttl: Duration,
    #[allow(dead_code)]
    pub reservation_expiry_sweep: Duration,
    pub dual_write_enabled: bool,
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] pub kafka_producer: FutureProducer,
    pub metrics: Arc<InventoryMetrics>,
}

// Metrics implementation now provided by common-observability crate.

async fn metrics_endpoint(State(state): State<AppState>) -> (axum::http::StatusCode, String) {
    let encoder = TextEncoder::new();
    let families = state.metrics.registry.gather();
    let mut buf = Vec::new();
    if let Err(e) = encoder.encode(&families, &mut buf) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("metrics encode error: {e}"),
        );
    }
    (
        axum::http::StatusCode::OK,
        String::from_utf8_lossy(&buf).to_string(),
    )
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
    log_rounding_mode_once();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db_pool = PgPool::connect(&database_url).await?;

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .set("group.id", "inventory-service")
        .set("enable.auto.commit", "true")
        .create()
        .expect("failed to create kafka consumer");
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    consumer.subscribe(&[
        "order.completed",
        "order.voided",
        "payment.completed",
        "product.created",
    ])?;

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .create()
        .expect("failed to create kafka producer");

    let multi_location_enabled = env::var("MULTI_LOCATION_ENABLED")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let reservation_default_ttl = env::var("RESERVATION_DEFAULT_TTL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(DEFAULT_RESERVATION_TTL_SECS as u64));
    let reservation_expiry_sweep = env::var("RESERVATION_EXPIRY_SWEEP_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(60));

    let dual_write_enabled = env::var("INVENTORY_DUAL_WRITE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let metrics = Arc::new(InventoryMetrics::new());
    let state = AppState {
        db: db_pool.clone(),
        jwt_verifier,
        multi_location_enabled,
        reservation_default_ttl: reservation_default_ttl,
        reservation_expiry_sweep: reservation_expiry_sweep,
        dual_write_enabled,
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] kafka_producer: producer.clone(),
        metrics: metrics.clone(),
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

    // Error metrics middleware using dedicated state (Arc<InventoryMetrics>) passed via from_fn_with_state.
    async fn error_metrics_mw(
        State(metrics): State<Arc<InventoryMetrics>>,
        req: axum::http::Request<Body>,
        next: middleware::Next,
    ) -> axum::response::Response {
        let resp = next.run(req).await;
        let status = resp.status();
        if status.as_u16() >= 400 {
            let code = resp
                .headers()
                .get("x-error-code")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown");
            metrics
                .http_errors_total
                .with_label_values(&["inventory-service", code, status.as_str()])
                .inc();
        }
        resp
    }

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/inventory", get(list_inventory))
        .route("/inventory/reservations", post(create_reservation))
        .route(
            "/inventory/reservations/:order_id",
            delete(release_reservation),
        )
        .route("/locations", get(list_locations))
        .route("/metrics", get(metrics_endpoint))
    .with_state(state.clone())
    .layer(middleware::from_fn_with_state(metrics.clone(), error_metrics_mw))
        .layer(cors);

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    {
        let db_for_consumer = db_pool.clone();
        let multi_loc_for_consumer = state.multi_location_enabled;
        let producer = producer.clone();
        tokio::spawn(async move {
            let mut stream = consumer.stream();
            while let Some(message) = stream.next().await {
                match message {
                    Ok(m) => {
                        let topic = m.topic();
                        if let Some(Ok(text)) = m.payload_view::<str>() {
                            if topic == "order.completed" {
                                handle_order_completed(text, &db_for_consumer, &producer, multi_loc_for_consumer).await;
                            } else if topic == "order.voided" {
                                #[cfg(any(feature = "kafka", feature = "kafka-producer"))] {
                                    handle_order_voided(text, &db_for_consumer).await;
                                }
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
    }

    // Spawn reservation expiration sweeper
    spawn_reservation_sweeper(state.clone());

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

#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
async fn handle_order_completed(text: &str, db: &PgPool, producer: &FutureProducer, multi_location_enabled: bool) {
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
                if multi_location_enabled {
                    // Use dynamic queries (query / query_as) to avoid sqlx compile-time validation failing before migrations.
                    if let Ok(res_rows) = sqlx::query(
                        "SELECT location_id, quantity FROM inventory_reservations WHERE order_id = $1 AND tenant_id = $2 AND product_id = $3"
                    )
                    .bind(order_id)
                    .bind(tenant_id)
                    .bind(product_id)
                    .fetch_all(&mut *tx)
                    .await
                    {
                        for r in res_rows.iter() {
                            let loc_id: Option<Uuid> = r.get("location_id");
                            let q: i32 = r.get("quantity");
                            if let Some(loc) = loc_id {
                                if let Err(err) = sqlx::query(
                                    "UPDATE inventory_items SET quantity = quantity - $1, updated_at = NOW() WHERE tenant_id = $2 AND product_id = $3 AND location_id = $4"
                                )
                                .bind(q)
                                .bind(tenant_id)
                                .bind(product_id)
                                .bind(loc)
                                .execute(&mut *tx)
                                .await {
                                    tracing::error!(?err, product_id = %product_id, tenant_id = %tenant_id, location_id = %loc, "Failed to decrement inventory_items for completion");
                                }
                            }
                        }
                    }
                    if let Ok(row) = sqlx::query(
                        "SELECT COALESCE(SUM(quantity),0) as quantity, MIN(threshold) as threshold FROM inventory_items WHERE tenant_id = $1 AND product_id = $2"
                    )
                    .bind(tenant_id)
                    .bind(product_id)
                    .fetch_one(&mut *tx)
                    .await {
                        let q: i64 = row.get("quantity");
                        let th: Option<i32> = row.get::<Option<i32>, _>("threshold");
                        if let Some(thr) = th { latest = Some((q as i32, thr)); }
                    }
                } else {
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

            #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
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

            // Dual-write validation: verify legacy aggregate matches sum of multi-location if both features active.
            if multi_location_enabled {
                if let Ok(rows) = sqlx::query(
                    "SELECT product_id, SUM(quantity) as sum_qty FROM inventory_items WHERE tenant_id = $1 GROUP BY product_id"
                )
                .bind(tenant_id)
                .fetch_all(db)
                .await
                {
                    for r in rows {
                        let product_id: Uuid = r.get("product_id");
                        let sum_qty: Option<i64> = r.get("sum_qty");
                        if let Some(sum_qty) = sum_qty {
                            if let Ok(legacy) = sqlx::query(
                                "SELECT quantity FROM inventory WHERE tenant_id = $1 AND product_id = $2"
                            )
                            .bind(tenant_id)
                            .bind(product_id)
                            .fetch_optional(db)
                            .await
                            {
                                if let Some(row) = legacy {
                                    let legacy_qty: i32 = row.get("quantity");
                                    if legacy_qty != sum_qty as i32 {
                                        tracing::warn!(product_id = %product_id, tenant_id = %tenant_id, legacy = legacy_qty, agg = sum_qty, "Dual-write divergence detected");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(err) => tracing::error!(?err, "Failed to parse OrderCompletedEvent"),
    }
}

#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
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

#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
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

fn spawn_reservation_sweeper(state: AppState) {
    tokio::spawn(async move {
        let sweep_interval = state.reservation_expiry_sweep;
        loop {
            tokio::time::sleep(sweep_interval).await;
            let start = std::time::Instant::now();
            if let Err(err) = expire_reservations(&state).await {
                tracing::error!(?err, "Reservation sweeper error");
            }
            let elapsed = start.elapsed().as_secs_f64();
            state.metrics.sweeper_duration_seconds.observe(elapsed);
        }
    });
}

async fn expire_reservations(state: &AppState) -> anyhow::Result<()> {
    // Update expired reservations and restock inventory for multi-location aware system.
    let mut tx = state.db.begin().await?;
    let rows = sqlx::query(
        "UPDATE inventory_reservations SET status = 'EXPIRED' WHERE status = 'ACTIVE' AND expires_at IS NOT NULL AND expires_at < NOW() RETURNING product_id, tenant_id, location_id, quantity, order_id"
    )
    .fetch_all(&mut *tx)
    .await?;
    if !rows.is_empty() {
        for r in rows.iter() {
            let product_id: Uuid = r.get("product_id");
            let tenant_id: Uuid = r.get("tenant_id");
            let quantity: i32 = r.get("quantity");
            let order_id: Uuid = r.get("order_id");
            if state.multi_location_enabled {
                let loc_id: Option<Uuid> = r.get("location_id");
                if let Some(loc) = loc_id {
                    let _ = sqlx::query(
                        "UPDATE inventory_items SET quantity = quantity + $1, updated_at = NOW() WHERE tenant_id = $2 AND product_id = $3 AND location_id = $4"
                    )
                    .bind(quantity)
                    .bind(tenant_id)
                    .bind(product_id)
                    .bind(loc)
                    .execute(&mut *tx)
                    .await;
                }
            } else {
                let _ = sqlx::query(
                    "UPDATE inventory SET quantity = quantity + $1 WHERE tenant_id = $2 AND product_id = $3"
                )
                .bind(quantity)
                .bind(tenant_id)
                .bind(product_id)
                .execute(&mut *tx)
                .await;
            }

            // Emit reservation expired event
            let expired_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let _evt = serde_json::json!({
                "type": "reservation.expired",
                "tenant_id": tenant_id,
                "product_id": product_id,
                "order_id": order_id,
                "quantity": quantity,
                "expired_at_epoch": expired_at,
            });
            #[cfg(feature = "kafka")]
            if let Err(err) = state.kafka_producer.send(
                rdkafka::producer::FutureRecord::to("inventory.reservation.expired")
                    .payload(&_evt.to_string())
                    .key(&tenant_id.to_string()),
                Duration::from_secs(0)
            ).await {
                tracing::error!(?err, tenant_id = %tenant_id, order_id = %order_id, "Failed to emit inventory.reservation.expired");
            }
            // Audit event
            let _audit_evt = serde_json::json!({
                "action": "inventory.reservation.expired",
                "schema_version": 1,
                "tenant_id": tenant_id,
                "order_id": order_id,
                "product_id": product_id,
                "quantity": quantity,
                "expired_at_epoch": expired_at,
            });
            #[cfg(feature = "kafka")] let _ = state.kafka_producer.send(
                rdkafka::producer::FutureRecord::to("audit.events")
                    .payload(&_audit_evt.to_string())
                    .key(&tenant_id.to_string()),
                Duration::from_secs(0)
            ).await;
        }
    }
    tx.commit().await?;

    // Dual-write validation (periodic) if enabled
    if state.multi_location_enabled && state.dual_write_enabled {
        if let Ok(tenants) = sqlx::query("SELECT DISTINCT tenant_id FROM inventory_items")
            .fetch_all(&state.db)
            .await
        {
            for r in tenants {
                let tenant_id: Uuid = r.get("tenant_id");
                if let Ok(rows) = sqlx::query(
                    "SELECT product_id, SUM(quantity) as sum_qty FROM inventory_items WHERE tenant_id = $1 GROUP BY product_id"
                )
                .bind(tenant_id)
                .fetch_all(&state.db)
                .await
                {
                    for row in rows {
                        let product_id: Uuid = row.get("product_id");
                        let sum_qty: Option<i64> = row.get("sum_qty");
                        if let Some(sum_qty) = sum_qty {
                            if let Ok(legacy) = sqlx::query(
                                "SELECT quantity FROM inventory WHERE tenant_id = $1 AND product_id = $2"
                            )
                            .bind(tenant_id)
                            .bind(product_id)
                            .fetch_optional(&state.db)
                            .await
                            {
                                if let Some(lrow) = legacy {
                                    let legacy_qty: i32 = lrow.get("quantity");
                                    if legacy_qty != sum_qty as i32 {
                                        tracing::warn!(product_id = %product_id, tenant_id = %tenant_id, legacy = legacy_qty, agg = sum_qty, "Dual-write divergence detected (sweeper)");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
