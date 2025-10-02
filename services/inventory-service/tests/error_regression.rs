use axum::{Router, routing::get, http::{Request, StatusCode, HeaderValue}};
use inventory_service::inventory_handlers::list_inventory;
use inventory_service::AppState;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use common_observability::InventoryMetrics;
use tower::ServiceExt;
use common_auth::{JwtVerifier, JwtConfig};

async fn test_state() -> AppState {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://postgres:postgres@localhost:5432/inventory_tests")
        .expect("lazy pool ok");
    AppState {
        db: pool,
        jwt_verifier: Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud"))),
        multi_location_enabled: false,
        reservation_default_ttl: std::time::Duration::from_secs(900),
        reservation_expiry_sweep: std::time::Duration::from_secs(60),
        dual_write_enabled: false,
        #[cfg(feature = "kafka")] kafka_producer: rdkafka::ClientConfig::new().set("bootstrap.servers","localhost:9092").create().unwrap(),
        metrics: Arc::new(InventoryMetrics::new()),
    }
}

#[tokio::test]
async fn missing_tenant_returns_400() {
    let state = test_state().await;
    let app = Router::new().route("/inventory", get(list_inventory)).with_state(state);
    let req = Request::builder().uri("/inventory").method("GET").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_tenant_id");
}

#[tokio::test]
async fn forbidden_role_returns_403() {
    let state = test_state().await;
    let app = Router::new().route("/inventory", get(list_inventory)).with_state(state);
    let mut req = Request::builder().uri("/inventory").method("GET").body(axum::body::Body::empty()).unwrap();
    {
        let h = req.headers_mut();
        h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
        h.insert("X-Roles", HeaderValue::from_static("support"));
        h.insert("X-User-ID", HeaderValue::from_static("22222222-2222-2222-2222-222222222222"));
    }
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
}
