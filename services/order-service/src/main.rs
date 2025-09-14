use axum::{routing::{get, post}, Router, Json};
use axum::extract::State;
use tokio::net::TcpListener;
use std::net::SocketAddr;
use sqlx::PgPool;
use uuid::Uuid;
use std::env;
mod order_handlers;
use order_handlers::{create_order, list_orders};
/// Shared application state
pub struct AppState {
    db: PgPool,
    kafka_producer: rdkafka::producer::FutureProducer
}

async fn health() -> &'static str { "ok" }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    // Initialize database connection pool
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db = PgPool::connect(&database_url).await?;
    // Initialize Kafka producer (assumes local broker)
    let producer: rdkafka::producer::FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()))
        .create()
        .expect("failed to create kafka producer");
    // Build application state
    let state = AppState { db: db, kafka_producer: producer };
    // Build application routes with Axum
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/orders", post(create_order).get(list_orders))
        .with_state(state);
    // Start server
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8084);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));
    println!("starting order-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
