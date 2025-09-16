use axum::{routing::get, Router};
use tokio::net::TcpListener;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use uuid::Uuid;
use futures_util::StreamExt;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::Message;
use std::env;

mod analytics_handlers;
use analytics_handlers::get_summary;

/// Aggregated metrics for a tenant
#[derive(Default, Clone, Copy, serde::Serialize)]
pub struct Stats {
    total_sales: f64,
    order_count: u64,
}

/// Application state holding the in-memory analytics store
#[derive(Clone)]
pub struct AppState {
    pub data: Arc<Mutex<HashMap<Uuid, Stats>>>,
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let store: Arc<Mutex<HashMap<Uuid, Stats>>> = Arc::new(Mutex::new(HashMap::new()));
    let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()))
        .set("group.id", "analytics-service")
        .set("enable.auto.commit", "true")
        .create()
        .expect("failed to create kafka consumer");
    consumer.subscribe(&["order.completed"])?;

    let data_ref = Arc::clone(&store);
    tokio::spawn(async move {
        let mut stream = consumer.stream();
        while let Some(message) = stream.next().await {
            if let Ok(m) = message {
                if let Some(Ok(text)) = m.payload_view::<str>() {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
                        if let (Some(tid_str), Some(total_val)) = (val.get("tenant_id"), val.get("total")) {
                            if let (Some(tid_str), Some(total)) = (tid_str.as_str(), total_val.as_f64()) {
                                if let Ok(tenant_id) = Uuid::parse_str(tid_str) {
                                    let mut map = data_ref.lock().unwrap();
                                    let entry = map.entry(tenant_id).or_insert_with(Stats::default);
                                    entry.total_sales += total;
                                    entry.order_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    let state = AppState { data: store };
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/analytics", get(get_summary))
        .with_state(state);

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8082);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));

    println!("starting analytics-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
