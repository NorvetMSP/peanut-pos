use axum::{routing::get, Router};
use product_service::app_state::AppState;
use product_service::product_handlers::list_products; // already requires security ctx
// No direct security extractor import needed for this test; route handler enforces roles internally.
use common_auth::{JwtConfig, JwtVerifier};
use sqlx::PgPool;
use std::{env, sync::Arc};
use tower::util::ServiceExt;
use uuid::Uuid;
use hyper::Request;
use axum::body::Body;
use http_body_util::BodyExt;
#[cfg(feature = "kafka")] use rdkafka::producer::FutureProducer;
#[cfg(not(feature = "kafka"))]
#[test]
fn skipped_auth_error_shape_without_kafka() { eprintln!("skipped auth_error_shape without kafka feature"); }

async fn dummy_verifier() -> Arc<JwtVerifier> {
    // Avoid ambiguous Into<String> inference by passing plain &str literals
    Arc::new(JwtVerifier::new(JwtConfig::new("http://issuer", "aud")))
}

#[tokio::test]
async fn unified_auth_error_shape() {
    let db_url = match env::var("TEST_AUDIT_DB_URL") { Ok(v) => v, Err(_) => { eprintln!("skipping: TEST_AUDIT_DB_URL not set"); return; } };
    let pool = PgPool::connect(&db_url).await.unwrap();
    #[cfg(feature = "kafka")]
    let kafka: FutureProducer = rdkafka::ClientConfig::new().set("bootstrap.servers","localhost:9092").create().unwrap();
    #[cfg(not(feature = "kafka"))]
    let kafka = (); // placeholder
    let verifier = dummy_verifier().await;
    let state = AppState::new(pool, kafka, verifier, None);

    // Minimal route just reusing list_products which enforces Admin/Manager
    let app = Router::new().route("/products", get(list_products)).with_state(state);

    // Build request with Support role only (should fail role check)
    let req = Request::builder()
        .uri("/products")
        .header("X-Tenant-ID", Uuid::new_v4().to_string())
        .header("X-User-ID", Uuid::new_v4().to_string())
        .header("X-Roles", "Support")
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("\"code\":\"missing_role\""));
    assert!(text.contains("Manager"));
}
