use loyalty_service::{AppState, get_points};
use axum::{Router, routing::get};
use axum::http::{Request, StatusCode, HeaderValue};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use common_auth::{JwtVerifier, JwtConfig};
use tower::ServiceExt;
use uuid::Uuid;

// Minimal wrapper to re-use get_points via direct route (not exported; mimic path)
async fn build_app() -> Router {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://postgres:postgres@localhost:5432/loyalty_tests")
        .expect("lazy pool");
    let verifier = Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud")));
    let state = AppState { db: pool, jwt_verifier: verifier, #[cfg(feature="kafka")] producer: {
        #[cfg(feature="kafka")] {
            use rdkafka::producer::FutureProducer; use rdkafka::ClientConfig; ClientConfig::new().set("bootstrap.servers","localhost:9092").create().unwrap()
        }
    }};
    Router::new().route("/points", get(get_points)).with_state(state)
}

#[tokio::test]
async fn points_missing_tenant_header() {
    let app = build_app().await;
    let req = Request::builder().uri("/points?customer_id=".to_string() + &Uuid::new_v4().to_string())
        .method("GET").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn points_forbidden_role() {
    let app = build_app().await;
    let cust_id = Uuid::new_v4();
    let mut req = Request::builder().uri(format!("/points?customer_id={cust_id}")).method("GET")
        .body(axum::body::Body::empty()).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", HeaderValue::from_str(&Uuid::new_v4().to_string()).unwrap());
    headers.insert("X-Roles", HeaderValue::from_static("support")); // unsupported role
    headers.insert("X-User-ID", HeaderValue::from_str(&Uuid::new_v4().to_string()).unwrap());
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
