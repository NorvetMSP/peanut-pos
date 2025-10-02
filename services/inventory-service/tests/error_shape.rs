use axum::response::IntoResponse; // bring trait into scope
use axum::extract::State;
use http_body_util::BodyExt; // for collect()
use uuid::Uuid;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use common_auth::{JwtConfig, JwtVerifier, Claims, AuthContext};
use common_observability::InventoryMetrics;
use inventory_service::inventory_handlers::{list_inventory, InventoryQueryParams};
use inventory_service::AppState;

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

    // Construct auth context Claims
    let claims = Claims {
        subject: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        roles: vec!["admin".to_string()],
        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        issued_at: Some(chrono::Utc::now()),
        issuer: "issuer".to_string(),
        audience: vec!["aud".to_string()],
        raw: serde_json::json!({}),
    };
    let auth = AuthContext { claims, token: "t".into() };
    let result = list_inventory(State(state), auth, axum::http::HeaderMap::new(), axum::extract::Query(InventoryQueryParams::default())).await;
    let err = result.expect_err("expected error");
    let resp = err.into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    // Collect the body (hyper 1.0 pattern via http-body-util)
    let collected = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(collected.to_vec()).unwrap();
    assert!(text.contains("missing_tenant_header"));
    assert!(text.contains("code"));
}
