mod analytics_handlers;

use analytics_handlers::{get_anomalies, get_forecast, get_summary};
use axum::{routing::get, Router};
use futures_util::StreamExt;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::Message;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use uuid::Uuid;

#[derive(Default, Clone, Copy, serde::Serialize)]
pub struct Stats {
    total_sales: f64,
    order_count: u64,
    refund_amount: f64,
    refund_count: u64,
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub data: Arc<Mutex<HashMap<Uuid, Stats>>>,
    pub product_counts: Arc<Mutex<HashMap<Uuid, HashMap<Uuid, i32>>>>,
}

#[derive(serde::Serialize)]
struct AnalyticsAlertEvent {
    tenant_id: Uuid,
    alert_type: String,
    details: String,
}

#[derive(serde::Deserialize)]
struct LowStockEvent {
    tenant_id: Uuid,
    product_id: Uuid,
    quantity: i32,
    threshold: i32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let database_url = std::env::var("DATABASE_URL")?;
    let db = PgPool::connect(&database_url).await?;

    let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &std::env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .set("group.id", "analytics-service")
        .set("enable.auto.commit", "true")
        .create()
        .expect("failed to create kafka consumer");
    consumer.subscribe(&["order.completed", "inventory.low_stock"])?;

    let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            &std::env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .create()
        .expect("failed to create kafka producer");

    let data_map = Arc::new(Mutex::new(HashMap::<Uuid, Stats>::new()));
    let product_counts_map = Arc::new(Mutex::new(HashMap::<Uuid, HashMap<Uuid, i32>>::new()));
    let db_pool = db.clone();
    let data_ref = Arc::clone(&data_map);
    let product_counts_ref = Arc::clone(&product_counts_map);
    let alert_producer = producer.clone();
    tokio::spawn(async move {
        let mut stream = consumer.stream();
        while let Some(message) = stream.next().await {
            if let Ok(m) = message {
                if let Some(Ok(text)) = m.payload_view::<str>() {
                    let topic = m.topic();
                    if topic == "order.completed" {
                        if let Ok(val) = serde_json::from_str::<Value>(text) {
                            if let (Some(tid_str), Some(total_val)) =
                                (val.get("tenant_id"), val.get("total"))
                            {
                                if let (Some(tid), Some(total)) =
                                    (tid_str.as_str(), total_val.as_f64())
                                {
                                    if let Ok(tenant_id) = Uuid::parse_str(tid) {
                                        // Update product counts for top 5 items
                                        if let Some(items_val) = val.get("items") {
                                            if let Some(arr) = items_val.as_array() {
                                                let mut counts = product_counts_ref.lock().unwrap();
                                                let tenant_counts = counts
                                                    .entry(tenant_id)
                                                    .or_insert_with(HashMap::new);
                                                for item in arr {
                                                    if let Some(pid_str) = item
                                                        .get("product_id")
                                                        .and_then(|v| v.as_str())
                                                    {
                                                        if let Some(qty) = item
                                                            .get("quantity")
                                                            .and_then(|v| v.as_i64())
                                                        {
                                                            if let Ok(pid) =
                                                                Uuid::parse_str(pid_str)
                                                            {
                                                                *tenant_counts
                                                                    .entry(pid)
                                                                    .or_insert(0) += qty as i32;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        let (sales_inc, orders_inc, refunds_inc, refund_count_inc) = {
                                            let mut map = data_ref.lock().unwrap();
                                            let entry = map.entry(tenant_id).or_default();
                                            if total < 0.0 {
                                                entry.refund_count += 1;
                                                entry.refund_amount += -total;
                                            } else {
                                                entry.order_count += 1;
                                                entry.total_sales += total;
                                            }
                                            let sales_inc = if total > 0.0 { total } else { 0.0 };
                                            let orders_inc = if total > 0.0 { 1 } else { 0 };
                                            let refunds_inc =
                                                if total < 0.0 { -total } else { 0.0 };
                                            let refund_count_inc = if total < 0.0 { 1 } else { 0 };
                                            (sales_inc, orders_inc, refunds_inc, refund_count_inc)
                                        };
                                        let query = r#"INSERT INTO daily_sales
                                                (tenant_id, date, total_sales, order_count, refund_amount, refund_count)
                                            VALUES ($1, CURRENT_DATE, $2, $3, $4, $5)
                                            ON CONFLICT (tenant_id, date)
                                            DO UPDATE
                                               SET total_sales = daily_sales.total_sales + $2,
                                                   order_count = daily_sales.order_count + $3,
                                                   refund_amount = daily_sales.refund_amount + $4,
                                                   refund_count = daily_sales.refund_count + $5"#;
                                        let _ = sqlx::query(query)
                                            .bind(tenant_id)
                                            .bind(sales_inc)
                                            .bind(orders_inc)
                                            .bind(refunds_inc)
                                            .bind(refund_count_inc)
                                            .execute(&db_pool)
                                            .await;

                                        if refunds_inc > 0.0 {
                                            if let Ok(avg_refund_opt) =
                                                sqlx::query_scalar::<_, Option<f64>>(
                                                    "SELECT AVG(refund_amount) FROM daily_sales \
                                                 WHERE tenant_id = $1 AND date < CURRENT_DATE",
                                                )
                                                .bind(tenant_id)
                                                .fetch_one(&db_pool)
                                                .await
                                            {
                                                let avg_refund = avg_refund_opt.unwrap_or(0.0);
                                                if avg_refund > 0.0
                                                    && refunds_inc > 2.0 * avg_refund
                                                {
                                                    let alert = AnalyticsAlertEvent {
                                                        tenant_id,
                                                        alert_type: "HIGH_REFUND_VOLUME".into(),
                                                        details: format!(
                                                            "${:.2} refunded today vs ${:.2} avg",
                                                            refunds_inc, avg_refund
                                                        ),
                                                    };
                                                    let payload =
                                                        serde_json::to_string(&alert).unwrap();
                                                    if let Err(err) = alert_producer
                                                        .send(
                                                            FutureRecord::to("analytics.alert")
                                                                .payload(&payload)
                                                                .key(&tenant_id.to_string()),
                                                            Duration::from_secs(0),
                                                        )
                                                        .await
                                                    {
                                                        tracing::error!(
                                                            "Failed to publish analytics.alert: {:?}",
                                                            err
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if topic == "inventory.low_stock" {
                        if let Ok(evt) = serde_json::from_str::<LowStockEvent>(text) {
                            tracing::warn!(
                                "Low stock alert: product {} is low (qty {} <= threshold {}) for tenant {}",
                                evt.product_id,
                                evt.quantity,
                                evt.threshold,
                                evt.tenant_id
                            );
                        }
                    }
                }
            }
        }
    });

    let state = AppState {
        db: db.clone(),
        data: data_map.clone(),
        product_counts: product_counts_map.clone(),
    };
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/analytics/forecast", get(get_forecast))
        .route("/analytics/anomalies", get(get_anomalies))
        .route("/analytics/summary", get(get_summary))
        .with_state(state);

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8082);
    let host_ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((host_ip, port));
    println!("starting analytics-service on {}", addr);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
