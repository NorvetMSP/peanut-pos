#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::post, extract::FromRef};
    use axum::http::Request;
    use axum::body::{Body, to_bytes};
    use tower::util::ServiceExt;
    use common_auth::{JwtVerifier, JwtConfig};
    use std::sync::Arc;

    impl FromRef<AppState> for Arc<JwtVerifier> { fn from_ref(s:&AppState)->Self{s.jwt_verifier.clone()} }

    #[tokio::test]
    async fn unauthorized_flow_returns_json_envelope(){
        let cfg = JwtConfig::new("issuer".into(), "aud".into());
        let verifier = JwtVerifier::builder(cfg).build().await.expect("build verifier");
    let state = AppState { jwt_verifier: Arc::new(verifier), db: None };
        let app = Router::new()
            .route("/payments", post(crate::payment_handlers::process_card_payment))
            .with_state(state);
        let body = serde_json::json!({"orderId":"12345678","method":"card","amount":"10.00"}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/payments")
            .header("content-type","application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert!(resp.status().as_u16() >= 400);
        assert!(resp.headers().get("X-Error-Code").is_some(), "missing X-Error-Code header");
        let bytes = to_bytes(resp.into_body(), 1024*16).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("\"code\":"), "body missing code field: {}", text);
    }
}
