use axum::{
    routing::{get, post},
    Router,
};
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
use uuid::Uuid;

mod order_handlers;
use order_handlers::{create_order, list_orders, OrderItem};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub kafka_producer: FutureProducer,
    pub pending_orders: Option<Arc<Mutex<HashMap<Uuid, Vec<OrderItem>>>>>,
}

#[derive(serde::Deserialize, Debug)]
struct PaymentCompletedEvent {
    pub order_id: Uuid,
    pub tenant_id: Uuid,
    pub amount: f64,
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

    let pending_orders: Arc<Mutex<HashMap<Uuid, Vec<OrderItem>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let state = AppState {
        db: db.clone(),
        kafka_producer: kafka_producer.clone(),
        pending_orders: Some(pending_orders.clone()),
    };

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/orders", post(create_order).get(list_orders))
        .with_state(state);

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
            .subscribe(&["payment.completed"])
            .expect("failed to subscribe");
        let mut stream = consumer.stream();
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(m) => {
                    if let Some(Ok(payload)) = m.payload_view::<str>() {
                        if let Ok(evt) = serde_json::from_str::<PaymentCompletedEvent>(payload) {
                            let _ =
                                sqlx::query("UPDATE orders SET status = 'COMPLETED' WHERE id = $1")
                                    .bind(evt.order_id)
                                    .execute(&db_pool)
                                    .await;

                            let maybe_items = {
                                let mut map = pending_orders_consumer.lock().unwrap();
                                map.remove(&evt.order_id)
                            };

                            if let Some(items) = maybe_items {
                                let order_event = serde_json::json!({
                                    "order_id": evt.order_id,
                                    "tenant_id": evt.tenant_id,
                                    "items": items,
                                    "total": evt.amount
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
                                    tracing::error!("Failed to send order.completed: {:?}", err);
                                } else {
                                    tracing::info!(
                                        "Order {} marked COMPLETED (payment confirmed)",
                                        evt.order_id
                                    );
                                }
                            }
                        }
                    }
                }
                Err(err) => tracing::error!("Kafka consume error: {:?}", err),
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
