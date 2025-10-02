use axum::response::IntoResponse; // for converting ApiError
use http_body_util::BodyExt; // for collect()
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use common_auth::{JwtConfig, JwtVerifier};
use common_observability::InventoryMetrics;
use inventory_service::{AppState, list_inventory};
use axum::{Router, routing::get};
use tower::ServiceExt; // for oneshot
use axum::http::Request;

#[tokio::test]
async fn list_inventory_missing_tenant_header_error_shape() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://postgres:postgres@localhost:5432/inventory_tests")
        .expect("lazy pool");
    let verifier = Arc::new(JwtVerifier::new(JwtConfig::new("issuer", "aud")));
    #[cfg(feature = "kafka")]
    let producer: rdkafka::producer::FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", "localhost:9092")
        .create()
        .expect("producer");
    let state = AppState {
        db: pool,
        jwt_verifier: verifier,
        multi_location_enabled: false,
        reservation_default_ttl: std::time::Duration::from_secs(900),
        reservation_expiry_sweep: std::time::Duration::from_secs(60),
        dual_write_enabled: false,
        #[cfg(feature = "kafka")] kafka_producer: producer,
        metrics: Arc::new(InventoryMetrics::new()),
    };

    // Call through router without X-Tenant-ID header so extractor rejects
    let app = Router::new().route("/inventory", get(list_inventory)).with_state(state);
    let req = Request::builder().uri("/inventory").method("GET").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    // Collect the body (hyper 1.0 pattern via http-body-util)
    let collected = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(collected.to_vec()).unwrap();
    // Extractor returns plain string error from SecurityError::MissingTenant
    assert!(text.contains("missing tenant"), "body was: {text}");
}
