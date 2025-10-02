use axum::{Router, routing::post, http::{Request, StatusCode, HeaderValue}};
use common_security::test_request_headers; // macro
use payment_service::{payment_handlers::process_card_payment, AppState};
use common_auth::{JwtVerifier, JwtConfig};
use std::sync::Arc;
use tower::ServiceExt;

fn state() -> AppState {
    AppState { jwt_verifier: Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud"))) , #[cfg(feature="kafka")] audit_producer: None }
}

#[tokio::test]
async fn missing_tenant_400() {
    let app = Router::new().route("/payments", post(process_card_payment)).with_state(state());
    // Build JSON manually to avoid requiring Serialize on PaymentRequest
    let json_body = r#"{ "orderId": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "method": "card", "amount": "12.34" }"#;
    let req = Request::builder()
        .uri("/payments")
        .method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(json_body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_tenant_id");
}

#[tokio::test]
async fn forbidden_role_403() {
    let app = Router::new().route("/payments", post(process_card_payment)).with_state(state());
    let json_body = r#"{ "orderId": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "method": "card", "amount": "12.34" }"#;
    let mut req = Request::builder()
        .uri("/payments")
        .method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(json_body))
        .unwrap();
    test_request_headers!(req, roles="support", tenant="11111111-1111-1111-1111-111111111111", user="22222222-2222-2222-2222-222222222222");
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
    let req = Request::builder().uri("/boom").method("GET").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "internal_error");
}
