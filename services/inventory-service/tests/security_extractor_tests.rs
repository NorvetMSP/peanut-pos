use inventory_service::{list_inventory, AppState};
mod test_utils; // bring in tests/test_utils.rs
use test_utils::{ensure_inventory_schema, lazy_app_state};
use axum::http::{Request, StatusCode, HeaderValue};
use axum::{routing::get, Router};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use common_observability::InventoryMetrics;
use tower::ServiceExt; // for oneshot
use uuid::Uuid;
// SecurityCtxExtractor used indirectly via axum extractor

#[tokio::test]
async fn list_inventory_missing_tenant_header() {
    let state = lazy_app_state();

    // Real extractor path: simply call endpoint without required header
    let app = Router::new().route("/inventory", get(list_inventory)).with_state(state);
    let req = Request::builder().uri("/inventory").method("GET").body(axum::body::Body::empty()).unwrap();
    // Intentionally omit X-Tenant-ID -> extractor should 400 (rejection)
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.headers().get("X-Error-Code").unwrap(), "missing_tenant_id");
}

#[tokio::test]
async fn list_inventory_cross_tenant_forbidden_when_mismatch() {
    let state = lazy_app_state();

    let app = Router::new().route("/inventory", get(list_inventory)).with_state(state);

    let mut req = Request::builder().uri("/inventory").method("GET").body(axum::body::Body::empty()).unwrap();
    let headers = req.headers_mut();
    // Provide invalid / mismatched roles (none) but tenant header present
    headers.insert("X-Tenant-ID", HeaderValue::from_str(&Uuid::new_v4().to_string()).unwrap());
    headers.insert("X-Roles", HeaderValue::from_static("support")); // not allowed

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN, "unsupported role should be forbidden");
}

#[tokio::test]
async fn list_inventory_happy_path_empty_ok() {
    // This test requires a real database (schema creation + query). Gate with TEST_DATABASE_URL.
    let db_url = match std::env::var("TEST_DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("SKIP list_inventory_happy_path_empty_ok: TEST_DATABASE_URL not set");
            return; // soft skip when env not configured
        }
    };
    let pool = PgPoolOptions::new()
        .connect(&db_url)
        .await
        .expect("connect test db");
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

    ensure_inventory_schema(&state.db).await.expect("ensure schema");
    let app = Router::new().route("/inventory", get(list_inventory)).with_state(state);

    let tenant_id = Uuid::new_v4();
    let mut req = Request::builder().uri("/inventory").method("GET").body(axum::body::Body::empty()).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", HeaderValue::from_str(&tenant_id.to_string()).unwrap());
    headers.insert("X-Roles", HeaderValue::from_static("admin"));
    headers.insert("X-User-ID", HeaderValue::from_str(&Uuid::new_v4().to_string()).unwrap());

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "expected 200 OK with minimal schema present, got {}", resp.status());
}

// helper removed; tests use real extractor behavior
