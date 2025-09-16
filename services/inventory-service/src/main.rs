use axum::{routing::get, Router, Json};
use axum::extract::State;
use tokio::net::TcpListener;
use std::net::SocketAddr;
use std::env;
use futures::StreamExt;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::Message;
use sqlx::PgPool;
use uuid::Uuid;
use serde::Deserialize;
mod inventory_handlers;
use inventory_handlers::list_inventory;
/// Event data from Order Service (for deserialization)
#[derive(Deserialize)]
#[serde(ignore_unknown_fields)]
struct OrderCompletedEvent {
    tenant_id: Uuid,
    items: Vec<OrderItem>
}
#[derive(Deserialize)]
struct OrderItem {
    product_id: Uuid,
    quantity: i32
}
/// Shared application state
pub struct AppState {
    db: PgPool
}

async fn health() -> &'static str { "ok" }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    // Initialize database pool
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db_pool = PgPool::connect(&database_url).await?;
    // Initialize Kafka consumer and producer
    let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()))
        .set("group.id", "inventory-service")
        .set("enable.auto.commit", "true")
        .create()
        .expect("failed to create kafka consumer");
    consumer.subscribe(&["order.completed"])?;
    let producer: rdkafka::producer::FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()))
        .create()
        .expect("failed to create kafka producer");
    // Spawn background task to consume order.completed events
    let db = db_pool.clone();
    tokio::spawn(async move {
        let mut stream = consumer.stream();
        while let Some(message) = stream.next().await {
            match message {
                Ok(m) => {
                    if let Some(Ok(text)) = m.payload_view::<str>() {
                        if let Ok(event) = serde_json::from_str::<OrderCompletedEvent>(text) {
                            for item in event.items {
                                // Decrement inventory stock
                                let result = sqlx::query!("UPDATE inventory SET quantity = quantity - $1 WHERE product_id = $2 AND tenant_id = $3 RETURNING quantity, threshold",
                                    item.quantity,
                                    item.product_id,
                                    event.tenant_id
                                )
                                .fetch_optional(&db)
                                .await;
                                if let Ok(rec) = result {
                                    if let Some(rec) = rec {
                                        if rec.quantity <= rec.threshold {
                                            // Trigger low-stock alert
                                            let alert = serde_json::json!({
                                                "product_id": item.product_id,
                                                "tenant_id": event.tenant_id,
                                                "quantity": rec.quantity,
                                                "threshold": rec.threshold
                                            });
                                            let _ = producer.send(
                                                rdkafka::producer::FutureRecord::to("inventory.low_stock")
                                                    .payload(&alert.to_string())
                                                    .key(&event.tenant_id.to_string()),
                                                0
                                            ).await;
                                        }
                                    } else {
                                        eprintln!("No inventory record for product {} (tenant {})", item.product_id, event.tenant_id);
                                    }
                                } else if let Err(err) = result {
                                    eprintln!("Inventory DB error: {}", err);
                                }
                            }
                        } else {
                            eprintln!("Failed to parse OrderCompletedEvent");
                        }
                    }
                }
                Err(e) => eprintln!("Kafka consume error: {}", e)
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
    let port: u16 = env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8087);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));
    println!("starting inventory-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
