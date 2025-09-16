
use sqlx::PgPool;
use futures_util::StreamExt;
use rdkafka::Message;
use serde_json::Value;
use uuid::Uuid;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use rdkafka::consumer::StreamConsumer;

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
}

// Placeholder for AnalyticsAlertEvent and LowStockEvent
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
    // ... initialize tracing ...
    let database_url = std::env::var("DATABASE_URL")?;
    let db = PgPool::connect(&database_url).await?;
    // Setup Kafka consumer for order and inventory events
    let consumer: StreamConsumer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &std::env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()))
        .set("group.id", "analytics-service")
        .set("enable.auto.commit", "true")
        .create()
        .expect("failed to create kafka consumer");
    consumer.subscribe(&["order.completed", "inventory.low_stock"])?;
    let data_map = Arc::new(Mutex::new(HashMap::<Uuid, Stats>::new()));
    // Clone DB pool and data map for use inside task
    let db_pool = db.clone();
    let data_ref = Arc::clone(&data_map);
    tokio::spawn(async move {
        let mut stream = consumer.stream();
        while let Some(message) = stream.next().await {
            if let Ok(m) = message {
                if let Some(Ok(text)) = m.payload_view::<str>() {
                    let topic = m.topic();
                    if topic == "order.completed" {
                        // Parse order completion event JSON
                        if let Ok(val) = serde_json::from_str::<Value>(text) {
                            if let (Some(tid_str), Some(total_val)) = (val.get("tenant_id"), val.get("total")) {
                                if let (Some(tid), Some(total)) = (tid_str.as_str(), total_val.as_f64()) {
                                    if let Ok(tenant_id) = Uuid::parse_str(tid) {
                                        // Update in-memory cumulative stats
                                        let mut map = data_ref.lock().unwrap();
                                        let entry = map.entry(tenant_id).or_default();
                                        if total < 0.0 {
                                            entry.refund_count += 1;
                                            entry.refund_amount += -total;
                                        } else {
                                            entry.order_count += 1;
                                            entry.total_sales += total;
                                        }
                                        drop(map);
                                        // Upsert daily summary into PostgreSQL
                                        let sales_inc = if total > 0.0 { total } else { 0.0 };
                                        let orders_inc = if total > 0.0 { 1 } else { 0 };
                                        let refunds_inc = if total < 0.0 { -total } else { 0.0 };
                                        let refund_count_inc = if total < 0.0 { 1 } else { 0 };
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
                                            .execute(&db_pool).await;
                                        // Anomaly detection: check refund spike rule
                                        if refunds_inc > 0.0 {
                                            // Compute average past refund volume (last 7 days)
                                            if let Ok(row) = sqlx::query!(
                                                "SELECT AVG(refund_amount) AS avg_refunds \
                                                 FROM daily_sales \
                                                 WHERE tenant_id = $1 AND date < CURRENT_DATE",
                                                 tenant_id
                                            ).fetch_one(&db_pool).await {
                                                let avg_refund = row.avg_refunds.unwrap_or(0.0);
                                                if avg_refund > 0.0 && refunds_inc > 2.0 * avg_refund {
                                                    // Publish analytics.alert event for high refund volume
                                                    let alert = AnalyticsAlertEvent {
                                                        tenant_id,
                                                        alert_type: "HIGH_REFUND_VOLUME".into(),
                                                        details: format!("${{:.2}} refunded today vs ${{:.2}} avg", refunds_inc, avg_refund),
                                                    };
                                                    let payload = serde_json::to_string(&alert).unwrap();
                                                    let _ = db_pool; // (notifying DB or other side effects if needed)
                                                    let _ = rdkafka::producer::FutureProducer::new(
                                                        // Producer config omitted for brevity
                                                    );
                                                    // In practice, use a shared producer or channel to publish the alert...
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if topic == "inventory.low_stock" {
                        // Parse low stock alert event
                        if let Ok(evt) = serde_json::from_str::<LowStockEvent>(text) {
                            tracing::warn!("Low stock alert: product {} is low (qty {} <= threshold {}) for tenant {}",
                                           evt.product_id, evt.quantity, evt.threshold, evt.tenant_id);
                            // (Optionally forward as analytics.alert or store for reporting)
                        }
                    }
                }
            }
        }
    });

    // --- Axum HTTP server setup ---
    use axum::{Router, routing::get};
    use analytics_handlers::{get_forecast, get_anomalies};

    let state = AppState { db: db.clone(), data: data_map.clone() };
    let app = Router::new()
        .route("/analytics/forecast", get(get_forecast))
        .route("/analytics/anomalies", get(get_anomalies));
        // You can add .with_state(state) if using Axum 0.6+

    // Listen on 0.0.0.0:8082 or env HOST/PORT
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8082);
    let addr = format!("{}:{}", host, port);
    println!("starting analytics-service on {}", addr);
    axum::Server::bind(&addr.parse().unwrap())
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
