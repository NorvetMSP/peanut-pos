use axum::{Router, routing::post, http::{Request, StatusCode, HeaderValue}};
use payment_service::{payment_handlers::process_card_payment, AppState};
use common_auth::{JwtVerifier, JwtConfig};
use std::sync::Arc;
use tower::ServiceExt;

fn state() -> AppState {
    AppState { jwt_verifier: Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud"))) }
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
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("support"));
    h.insert("X-User-ID", HeaderValue::from_static("22222222-2222-2222-2222-222222222222"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
