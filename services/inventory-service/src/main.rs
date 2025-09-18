use axum::{routing::get, Router};
use futures::StreamExt;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::Message;
use serde::Deserialize;
use sqlx::{query, query_as, PgPool};
use std::{env, net::SocketAddr, time::Duration};
use tokio::net::TcpListener;
use uuid::Uuid;

mod inventory_handlers;
use inventory_handlers::list_inventory;

const DEFAULT_THRESHOLD: i32 = 5;

#[derive(Deserialize)]
struct OrderCompletedEvent {
    tenant_id: Uuid,
    items: Vec<OrderItem>,
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
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    // Initialize database pool
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db_pool = PgPool::connect(&database_url).await?;

    // Initialize Kafka consumer and producer
    let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .set("group.id", "inventory-service")
        .set("enable.auto.commit", "true")
        .create()
        .expect("failed to create kafka consumer");
    consumer.subscribe(&["order.completed", "payment.completed", "product.created"])?;

    let producer: rdkafka::producer::FutureProducer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .create()
        .expect("failed to create kafka producer");

    // Spawn background task to consume events
    let db = db_pool.clone();

    tokio::spawn(async move {
        let mut stream = consumer.stream();
        while let Some(message) = stream.next().await {
            match message {
                Ok(m) => {
                    let topic = m.topic();
                    if let Some(Ok(text)) = m.payload_view::<str>() {
                        if topic == "order.completed" {
                            if let Ok(event) = serde_json::from_str::<OrderCompletedEvent>(text) {
                                for item in event.items {
                                    let product_id = item.product_id;
                                    let quantity_delta = item.quantity;
                                    let mut attempts = 0;
                                    let mut latest: Option<(i32, i32)> = None;

                                    loop {
                                        let update = query_as::<_, (i32, i32)>(
                                            "UPDATE inventory SET quantity = quantity - $1 WHERE product_id = $2 AND tenant_id = $3 RETURNING quantity, threshold",
                                        )
                                        .bind(quantity_delta)
                                        .bind(product_id)
                                        .bind(event.tenant_id)
                                        .fetch_optional(&db)
                                        .await;

                                        match update {
                                            Ok(Some(row)) => {
                                                latest = Some(row);
                                                break;
                                            }
                                            Ok(None) if attempts == 0 => {
                                                attempts += 1;
                                                if let Err(err) = query(
                                                    "INSERT INTO inventory (product_id, tenant_id, quantity, threshold) VALUES ($1, $2, $3, $4) ON CONFLICT (product_id, tenant_id) DO NOTHING",
                                                )
                                                .bind(product_id)
                                                .bind(event.tenant_id)
                                                .bind(0)
                                                .bind(DEFAULT_THRESHOLD)
                                                .execute(&db)
                                                .await
                                                {
                                                    eprintln!(
                                                        "Failed to initialize inventory record for product {} (tenant {}): {}",
                                                        product_id, event.tenant_id, err
                                                    );
                                                    break;
                                                }
                                                continue;
                                            }
                                            Ok(None) => {
                                                eprintln!(
                                                    "No inventory record for product {} (tenant {})",
                                                    product_id, event.tenant_id
                                                );
                                                break;
                                            }
                                            Err(err) => {
                                                eprintln!("Inventory DB error: {}", err);
                                                break;
                                            }
                                        }
                                    }

                                    if let Some((quantity, threshold)) = latest {
                                        if quantity <= threshold {
                                            let alert = serde_json::json!({
                                                "product_id": product_id,
                                                "tenant_id": event.tenant_id,
                                                "quantity": quantity,
                                                "threshold": threshold
                                            });
                                            let _ = producer
                                                .send(
                                                    rdkafka::producer::FutureRecord::to(
                                                        "inventory.low_stock",
                                                    )
                                                    .payload(&alert.to_string())
                                                    .key(&event.tenant_id.to_string()),
                                                    Duration::from_secs(0),
                                                )
                                                .await;
                                        }
                                    }
                                }
                            } else {
                                eprintln!("Failed to parse OrderCompletedEvent");
                            }
                        } else if topic == "product.created" {
                            if let Ok(event) = serde_json::from_str::<ProductCreatedEvent>(text) {
                                let initial_quantity = event.initial_quantity.unwrap_or(0);
                                let threshold = event.threshold.unwrap_or(DEFAULT_THRESHOLD);
                                if let Err(err) = query(
                                    "INSERT INTO inventory (product_id, tenant_id, quantity, threshold) VALUES ($1, $2, $3, $4) ON CONFLICT (product_id, tenant_id) DO NOTHING",
                                )
                                .bind(event.product_id)
                                .bind(event.tenant_id)
                                .bind(initial_quantity)
                                .bind(threshold)
                                .execute(&db)
                                .await
                                {
                                    eprintln!(
                                        "Failed to seed inventory for product {} (tenant {}): {}",
                                        event.product_id, event.tenant_id, err
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
                            } else {
                                eprintln!("Failed to parse ProductCreatedEvent");
                            }
                        } else if topic == "payment.completed" {
                            if let Ok(evt) = serde_json::from_str::<PaymentCompletedEvent>(text) {
                                tracing::info!(
                                    order_id = %evt.order_id,
                                    tenant_id = %evt.tenant_id,
                                    amount = evt.amount,
                                    "Inventory noticed payment.completed event"
                                );
                                // No inventory action needed here, order.completed handles stock updates
                            }
                        }
                    }
                }
                Err(e) => eprintln!("Kafka consume error: {}", e),
            }
        }
    });

    // Build and run the HTTP server (health endpoint and optional APIs)
    let state = AppState { db: db_pool };
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/inventory", get(list_inventory))
        .with_state(state);

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
