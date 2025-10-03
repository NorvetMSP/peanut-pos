use axum::{Router, routing::get, http::{Request, StatusCode, HeaderValue}};
use loyalty_service::{get_points, AppState};
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
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_tenant_id");
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
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
}

#[tokio::test]
async fn internal_error_500() {
    use axum::{routing::get};
    use common_http_errors::ApiError;
    async fn boom() -> Result<String, ApiError> { Err(ApiError::Internal { trace_id: None, message: Some("synthetic".into()) }) }
    let app = Router::new().route("/boom", get(boom));
    let req = Request::builder().uri("/boom").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "internal_error");
}
