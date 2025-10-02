use common_http_errors::ApiError;
use axum::response::IntoResponse;
use axum::body::to_bytes;

#[tokio::test]
async fn api_error_renders_standard_envelope() {
    let err = ApiError::BadRequest { code: "missing_customer_id", trace_id: None, message: Some("customer_id required".into()) };
    let resp = err.into_response();
    assert_eq!(resp.status().as_u16(), 400);
    let headers = resp.headers();
    assert_eq!(headers.get("X-Error-Code").unwrap(), "missing_customer_id");
    let body = to_bytes(resp.into_body(), 1024 * 8).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("\"code\":\"missing_customer_id\""), "unexpected body: {}", text);
}
