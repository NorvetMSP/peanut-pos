use axum::Router;
use common_money::log_rounding_mode_once;
use reqwest::Client;
use sqlx::PgPool;
use std::env;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
use std::sync::Arc;
use std::net::SocketAddr; 
use tokio::net::TcpListener;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
use uuid::Uuid;

// Reuse shared app builder and types from the library crate
use order_service::{AppState, build_router, build_jwt_verifier_from_env, spawn_jwks_refresh};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use futures_util::StreamExt;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::consumer::{Consumer, StreamConsumer};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::{FutureProducer, FutureRecord};
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::Message;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use std::time::Duration;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use bigdecimal::BigDecimal;

// Kafka-only event and row types used by the background consumer
#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
struct PaymentCompletedEvent { pub order_id: Uuid, pub tenant_id: Uuid, pub amount: BigDecimal }
#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
struct PaymentFailedEvent { pub order_id: Uuid, pub tenant_id: Uuid, pub method: String, pub reason: String }
#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct OrderFinancialSummary { total: Option<BigDecimal>, customer_id: Option<Uuid>, offline: bool, payment_method: String }
#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct OrderItemFinancialRow { product_id: Uuid, quantity: i32, unit_price: BigDecimal, line_total: BigDecimal }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    log_rounding_mode_once();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db = PgPool::connect(&database_url).await?;

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    let kafka_producer: FutureProducer = rdkafka::ClientConfig::new()
        .set(
            "bootstrap.servers",
            env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
        )
        .create()
        .expect("failed to create kafka producer");

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    let http_client = Client::new();
    let inventory_base_url =
        env::var("INVENTORY_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8087".to_string());

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    let state = AppState {
        db: db.clone(),
        kafka_producer: kafka_producer.clone(),
        jwt_verifier,
        http_client: http_client.clone(),
        inventory_base_url: inventory_base_url.clone(),
        audit_producer: Some(Arc::new(common_audit::BufferedAuditProducer::new(
            common_audit::AuditProducer::new(common_audit::KafkaAuditSink::new(
                kafka_producer.clone(),
                common_audit::AuditProducerConfig { topic: env::var("AUDIT_TOPIC").unwrap_or_else(|_| "audit.events".to_string()) }
            )),
            1024,
        ))),
    };
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    tracing::info!(topic = %env::var("AUDIT_TOPIC").unwrap_or_else(|_| "audit.events".to_string()), "Audit producer initialized");
    #[cfg(not(any(feature = "kafka", feature = "kafka-producer")))]
    let state = AppState {
        db: db.clone(),
        jwt_verifier,
        http_client: http_client.clone(),
        inventory_base_url: inventory_base_url.clone(),
    };

    // Build the HTTP app with shared router wiring (CORS, middleware, routes)
    let app: Router = build_router(state.clone());

    #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
    {
        let db_pool = db.clone();
        let producer = kafka_producer.clone();
        tokio::spawn(async move {
            let consumer: StreamConsumer = rdkafka::ClientConfig::new()
                .set(
                    "bootstrap.servers",
                    env::var("KAFKA_BOOTSTRAP").unwrap_or("localhost:9092".into()),
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
                                match serde_json::from_str::<PaymentCompletedEvent>(payload) {
                                    Ok(evt) => {
                                        if let Err(err) = sqlx::query(
                                            "UPDATE orders SET status = 'COMPLETED' WHERE id = $1 AND tenant_id = $2 AND status = 'PENDING'",
                                        )
                                        .bind(evt.order_id)
                                        .bind(evt.tenant_id)
                                        .execute(&db_pool)
                                        .await
                                        {
                                            tracing::error!(
                                                ?err,
                                                order_id = %evt.order_id,
                                                tenant_id = %evt.tenant_id,
                                                "Failed to update order status on payment completion"
                                            );
                                        }

                                        match sqlx::query_as::<_, OrderFinancialSummary>(
                                            "SELECT total::FLOAT8 as total, customer_id, offline, payment_method FROM orders WHERE id = $1 AND tenant_id = $2",
                                        )
                                        .bind(evt.order_id)
                                        .bind(evt.tenant_id)
                                        .fetch_optional(&db_pool)
                                        .await
                                        {
                                            Ok(Some(order_row)) => {
                                                match sqlx::query_as::<_, OrderItemFinancialRow>(
                                                    "SELECT product_id, quantity, unit_price::FLOAT8 as unit_price, line_total::FLOAT8 as line_total FROM order_items WHERE order_id = $1",
                                                )
                                                .bind(evt.order_id)
                                                .fetch_all(&db_pool)
                                                .await
                                                {
                                                    Ok(item_rows) => {
                                                        let event_items: Vec<serde_json::Value> = item_rows
                                                            .into_iter()
                                                            .map(|row| {
                                                                serde_json::json!({
                                                                    "product_id": row.product_id,
                                                                    "quantity": row.quantity,
                                                                    "unit_price": row.unit_price,
                                                                    "line_total": row.line_total,
                                                                })
                                                            })
                                                            .collect();

                                                        let event = serde_json::json!({
                                                            "order_id": evt.order_id,
                                                            "tenant_id": evt.tenant_id,
                                                            "items": event_items,
                                                            "total": order_row.total,
                                                            "customer_id": order_row.customer_id,
                                                            "offline": order_row.offline,
                                                            "payment_method": order_row.payment_method,
                                                        });

                                                        if let Err(err) = producer
                                                            .send(
                                                                FutureRecord::to("order.completed")
                                                                    .payload(&event.to_string())
                                                                    .key(&evt.tenant_id.to_string()),
                                                                Duration::from_secs(0),
                                                            )
                                                            .await
                                                        {
                                                            tracing::error!(
                                                                ?err,
                                                                order_id = %evt.order_id,
                                                                tenant_id = %evt.tenant_id,
                                                                "Failed to publish order.completed after payment confirmation"
                                                            );
                                                        } else {
                                                            tracing::info!(
                                                                order_id = %evt.order_id,
                                                                tenant_id = %evt.tenant_id,
                                                                "Order marked COMPLETED after payment confirmation"
                                                            );
                                                        }
                                                    }
                                                    Err(err) => {
                                                        tracing::error!(
                                                            ?err,
                                                            order_id = %evt.order_id,
                                                            tenant_id = %evt.tenant_id,
                                                            "Failed to load order items for payment completion"
                                                        );
                                                    }
                                                }
                                            }
                                            Ok(None) => {
                                                tracing::warn!(
                                                    order_id = %evt.order_id,
                                                    tenant_id = %evt.tenant_id,
                                                    "Payment completion received for unknown order"
                                                );
                                            }
                                            Err(err) => {
                                                tracing::error!(
                                                    ?err,
                                                    order_id = %evt.order_id,
                                                    tenant_id = %evt.tenant_id,
                                                    "Failed to load order for payment completion"
                                                );
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        tracing::error!(
                                            ?err,
                                            "Failed to parse PaymentCompletedEvent"
                                        );
                                    }
                                }
                            }
                                "payment.failed" => {
                                match serde_json::from_str::<PaymentFailedEvent>(payload) {
                                    Ok(evt) => {
                                        match sqlx::query(
                                            "UPDATE orders SET status = 'NOT_ACCEPTED' WHERE id = $1 AND tenant_id = $2 AND status = 'PENDING'",
                                        )
                                        .bind(evt.order_id)
                                        .bind(evt.tenant_id)
                                        .execute(&db_pool)
                                        .await
                                        {
                                            Ok(result) => {
                                                if result.rows_affected() == 0 {
                                                    tracing::warn!(
                                                        order_id = %evt.order_id,
                                                        tenant_id = %evt.tenant_id,
                                                        method = evt.method.as_str(),
                                                        reason = %evt.reason,
                                                        "Payment failure received but order already processed"
                                                    );
                                                } else {
                                                    tracing::warn!(
                                                        order_id = %evt.order_id,
                                                        tenant_id = %evt.tenant_id,
                                                        method = evt.method.as_str(),
                                                        reason = %evt.reason,
                                                        "Order marked NOT_ACCEPTED due to payment failure"
                                                    );

                                                    match sqlx::query_as::<_, OrderFinancialSummary>(
                                                        "SELECT total::FLOAT8 as total, customer_id, offline, payment_method FROM orders WHERE id = $1 AND tenant_id = $2",
                                                    )
                                                    .bind(evt.order_id)
                                                    .bind(evt.tenant_id)
                                                    .fetch_optional(&db_pool)
                                                    .await
                                                    {
                                                        Ok(Some(order_row)) => {
                                                            match sqlx::query_as::<_, OrderItemFinancialRow>(
                                                                "SELECT product_id, quantity, unit_price::FLOAT8 as unit_price, line_total::FLOAT8 as line_total FROM order_items WHERE order_id = $1",
                                                            )
                                                            .bind(evt.order_id)
                                                            .fetch_all(&db_pool)
                                                            .await
                                                            {
                                                                Ok(item_rows) => {
                                                                    let event_items: Vec<serde_json::Value> = item_rows
                                                                        .into_iter()
                                                                        .map(|row| {
                                                                            serde_json::json!({
                                                                                "product_id": row.product_id,
                                                                                "quantity": row.quantity,
                                                                                "unit_price": row.unit_price,
                                                                                "line_total": row.line_total,
                                                                            })
                                                                        })
                                                                        .collect();

                                                                    let void_reason = if evt.reason.is_empty() {
                                                                        Some(String::from("payment_failed"))
                                                                    } else {
                                                                        Some(format!("payment_failed: {}", evt.reason))
                                                                    };
                                                                    let void_event = serde_json::json!({
                                                                        "order_id": evt.order_id,
                                                                        "tenant_id": evt.tenant_id,
                                                                        "items": event_items,
                                                                        "total": order_row
                                                                            .total
                                                                            .clone()
                                                                            .unwrap_or_else(|| BigDecimal::from(0)),
                                                                        "customer_id": order_row.customer_id,
                                                                        "offline": order_row.offline,
                                                                        "payment_method": order_row.payment_method,
                                                                        "reason": void_reason,
                                                                    });

                                                                    if let Err(err) = producer
                                                                        .send(
                                                                            FutureRecord::to("order.voided")
                                                                                .payload(&void_event.to_string())
                                                                                .key(&evt.tenant_id.to_string()),
                                                                            Duration::from_secs(0),
                                                                        )
                                                                        .await
                                                                    {
                                                                        tracing::error!(
                                                                            ?err,
                                                                            order_id = %evt.order_id,
                                                                            tenant_id = %evt.tenant_id,
                                                                            "Failed to emit order.voided after payment failure"
                                                                        );
                                                                    }
                                                                }
                                                                Err(err) => {
                                                                    tracing::error!(
                                                                        ?err,
                                                                        order_id = %evt.order_id,
                                                                        tenant_id = %evt.tenant_id,
                                                                        "Failed to load order items after payment failure"
                                                                    );
                                                                }
                                                            }
                                                        }
                                                        Ok(None) => {
                                                            tracing::error!(
                                                                order_id = %evt.order_id,
                                                                tenant_id = %evt.tenant_id,
                                                                "Order missing when preparing void event after payment failure"
                                                            );
                                                        }
                                                        Err(err) => {
                                                            tracing::error!(
                                                                ?err,
                                                                order_id = %evt.order_id,
                                                                tenant_id = %evt.tenant_id,
                                                                "Failed to fetch order snapshot after payment failure"
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                            Err(err) => {
                                                tracing::error!(
                                                    ?err,
                                                    order_id = %evt.order_id,
                                                    tenant_id = %evt.tenant_id,
                                                    "Failed to update order for payment failure"
                                                );
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        tracing::error!(?err, "Failed to parse PaymentFailedEvent");
                                    }
                                }
                            }
                                _ => {}
                            }
                        }
                    }
                    Err(err) => tracing::error!(?err, "Kafka error"),
                }
            }
        });
    }

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

