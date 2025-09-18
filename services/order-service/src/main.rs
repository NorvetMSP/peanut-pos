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
use order_handlers::{create_order, list_orders, refund_order, OrderItem};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub kafka_producer: FutureProducer,
    pub pending_orders: Option<Arc<Mutex<HashMap<Uuid, (Vec<OrderItem>, Option<Uuid>, bool)>>>>,
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
    let state = AppState {
        db: db.clone(),
        kafka_producer: kafka_producer.clone(),
        pending_orders: Some(pending_orders.clone()),
    };

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/orders", post(create_order).get(list_orders))
        .route("/orders/refund", post(refund_order))
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
                                    let _ = sqlx::query("UPDATE orders SET status = 'COMPLETED' WHERE id = $1 AND tenant_id = $2 AND status <> 'REFUNDED'")
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
                                    match sqlx::query("UPDATE orders SET status = 'NOT_ACCEPTED' WHERE id = $1 AND tenant_id = $2 AND status = 'PENDING'")
                                        .bind(evt.order_id)
                                        .bind(evt.tenant_id)
                                        .execute(&db_pool)
                                        .await
                                    {
                                        Ok(result) => {
                                            if result.rows_affected() == 0 {
                                                tracing::warn!(order_id = %evt.order_id, tenant_id = %evt.tenant_id, method = evt.method.as_str(), reason = %evt.reason, "Payment failure received but order not updated");
                                            } else {
                                                tracing::warn!(order_id = %evt.order_id, tenant_id = %evt.tenant_id, method = evt.method.as_str(), reason = %evt.reason, "Order marked NOT_ACCEPTED due to payment failure");
                                            }
                                        }
                                        Err(err) => tracing::error!(?err, order_id = %evt.order_id, "Failed to update order status after payment failure"),
                                    }

                                    let removed = {
                                        let mut map = pending_orders_consumer.lock().unwrap();
                                        map.remove(&evt.order_id)
                                    };
                                    if removed.is_some() {
                                        tracing::debug!(order_id = %evt.order_id, method = evt.method.as_str(), "Removed pending order entry after payment failure");
                                    }
                                } else {
                                    tracing::error!("Failed to parse PaymentFailedEvent");
                                }
                            }
                            _ => {}
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
