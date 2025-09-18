// services/loyalty-service/src/main.rs
use axum::{
    extract::{Query, State},
    routing::get,
    Router,
};
use futures::StreamExt;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::Message;
use serde::Deserialize;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::net::TcpListener;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct CompletedEvent {
    order_id: Uuid,
    tenant_id: Uuid,
    total: f64,
    customer_id: Option<Uuid>,
}

#[derive(Clone)]
struct AppState {
    db: PgPool,
}

// GET /points?customer_id=... -> returns point balance
async fn get_points(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<String, (axum::http::StatusCode, String)> {
    let cust_id = params
        .get("customer_id")
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or((
            axum::http::StatusCode::BAD_REQUEST,
            "customer_id required".into(),
        ))?;

    // Query points for this customer
    let rec = sqlx::query("SELECT points FROM loyalty_points WHERE customer_id = $1")
        .bind(cust_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("DB error: {}", e),
            )
        })?;

    let points: i32 = rec.try_get("points").map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("DB error: {}", e),
        )
    })?;

    Ok(points.to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize DB and Kafka
    let database_url = env::var("DATABASE_URL")?;
    let db_pool = PgPool::connect(&database_url).await?;

    let bootstrap = env::var("KAFKA_BOOTSTRAP").unwrap_or_else(|_| "localhost:9092".into());
    let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &bootstrap)
        .set("group.id", "loyalty-service")
        .create()?;
    consumer.subscribe(&["order.completed"])?;

    let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &bootstrap)
        .create()?;

    let state = AppState {
        db: db_pool.clone(),
    };

    // Spawn task to handle events
    tokio::spawn({
        let db = db_pool.clone();
        let producer = producer.clone();
        async move {
            let mut stream = consumer.stream();
            while let Some(message) = stream.next().await {
                if let Ok(m) = message {
                    if let Some(Ok(text)) = m.payload_view::<str>() {
                        if let Ok(evt) = serde_json::from_str::<CompletedEvent>(text) {
                            if let Some(cust_id) = evt.customer_id {
                                // Calculate points delta
                                let delta = if evt.total >= 0.0 {
                                    evt.total.floor() as i32
                                } else {
                                    -(evt.total.abs().floor() as i32)
                                };

                                if delta != 0 {
                                    // Upsert into loyalty_points table
                                    let _ = sqlx::query(
                                        "INSERT INTO loyalty_points (customer_id, tenant_id, points)
                                         VALUES ($1, $2, $3)
                                         ON CONFLICT (customer_id) DO UPDATE
                                         SET points = loyalty_points.points + $3"
                                    )
                                    .bind(cust_id)
                                    .bind(evt.tenant_id)
                                    .bind(delta)
                                    .execute(&db)
                                    .await;
                                }

                                // Fetch new total
                                if let Ok(record) = sqlx::query(
                                    "SELECT points FROM loyalty_points WHERE customer_id = $1",
                                )
                                .bind(cust_id)
                                .fetch_one(&db)
                                .await
                                {
                                    if let Ok(new_balance) = record.try_get::<i32, _>("points") {
                                        // Emit loyalty.updated event
                                        let loyalty_event = serde_json::json!({
                                            "order_id": evt.order_id,
                                            "customer_id": cust_id,
                                            "tenant_id": evt.tenant_id,
                                            "points_delta": delta,
                                            "new_balance": new_balance
                                        });

                                        let _ = producer
                                            .send(
                                                FutureRecord::to("loyalty.updated")
                                                    .payload(&loyalty_event.to_string())
                                                    .key(&evt.tenant_id.to_string()),
                                                Duration::from_secs(0),
                                            )
                                            .await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    // HTTP server (for health and points query)
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/points", get(get_points))
        .with_state(state);

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8088);
    let ip: IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));
    println!("starting loyalty-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}
