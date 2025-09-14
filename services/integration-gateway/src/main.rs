use axum::{routing::{get, post}, Router, Json};
use axum::extract::State;
use tokio::net::TcpListener;
use std::net::SocketAddr;
use std::env;
use rdkafka::producer::FutureProducer;
mod integration_handlers;
use integration_handlers::process_payment;
/// Shared application state
pub struct AppState {
    kafka_producer: FutureProducer
}

async fn health() -> &'static str { "ok" }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    // Initialize Kafka producer for events
    let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()))
        .create()
        .expect("failed to create kafka producer");
    let state = AppState { kafka_producer: producer };
    // Build routes
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/payments", post(process_payment))
        .with_state(state);
    // Start server
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8083);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));
    println!("starting integration-gateway on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
