use axum::{Router, routing::get, http::{Request, StatusCode, HeaderValue}};
use loyalty_service::{api::get_points, api::AppState};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use common_auth::{JwtVerifier, JwtConfig};
use tower::ServiceExt;

async fn state() -> AppState {
    AppState {
        db: PgPoolOptions::new().connect_lazy("postgres://postgres:postgres@localhost:5432/loyalty_tests").unwrap(),
        jwt_verifier: Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud"))),
        #[cfg(feature = "kafka")]
        producer: rdkafka::ClientConfig::new().set("bootstrap.servers","localhost:9092").create().unwrap(),
    }
}

#[tokio::test]
async fn missing_tenant_400() {
    let app = Router::new().route("/points", get(get_points)).with_state(state().await);
    let req = Request::builder().uri("/points?customer_id=aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn forbidden_role_403() {
    let app = Router::new().route("/points", get(get_points)).with_state(state().await);
    let mut req = Request::builder().uri("/points?customer_id=aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").body(axum::body::Body::empty()).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("support"));
    h.insert("X-User-ID", HeaderValue::from_static("22222222-2222-2222-2222-222222222222"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
