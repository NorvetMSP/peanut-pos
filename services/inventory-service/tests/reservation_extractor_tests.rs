use inventory_service::{AppState, create_reservation};
use axum::{Router, routing::post};
use axum::http::{Request, HeaderValue, StatusCode};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use common_observability::InventoryMetrics;
use tower::ServiceExt;
use uuid::Uuid;
use serde_json::json;

#[tokio::test]
async fn create_reservation_rejects_empty_items() {
    let pool = PgPoolOptions::new().connect_lazy("postgres://postgres:postgres@localhost:5432/inventory_tests").unwrap();
    let jwt_verifier = Arc::new(common_auth::JwtVerifier::new(common_auth::JwtConfig::new("issuer","aud")));
    #[cfg(feature = "kafka")] let producer: rdkafka::producer::FutureProducer = rdkafka::ClientConfig::new().set("bootstrap.servers","localhost:9092").create().unwrap();
    let state = AppState {
        db: pool,
        jwt_verifier,
        multi_location_enabled: false,
        reservation_default_ttl: std::time::Duration::from_secs(900),
        reservation_expiry_sweep: std::time::Duration::from_secs(60),
        dual_write_enabled: false,
        #[cfg(feature = "kafka")] kafka_producer: producer,
        metrics: Arc::new(InventoryMetrics::new()),
    };

    let app = Router::new().route("/inventory/reservations", post(create_reservation)).with_state(state);

    let tenant_id = Uuid::new_v4();
    let body = json!({
        "order_id": Uuid::new_v4(),
        "items": []
    }).to_string();

    let mut req = Request::builder().uri("/inventory/reservations").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", HeaderValue::from_str(&tenant_id.to_string()).unwrap());
    headers.insert("X-Roles", HeaderValue::from_static("admin"));
    headers.insert("X-User-ID", HeaderValue::from_str(&Uuid::new_v4().to_string()).unwrap());

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "empty items should be rejected");
}
