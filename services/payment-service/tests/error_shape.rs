use common_http_errors::ApiError;
use axum::response::IntoResponse;
use axum::body::to_bytes;

#[tokio::test]
async fn macro_asserts_error_shape(){
    let err = ApiError::ForbiddenMissingRole { role: "payment_access", trace_id: None };
    let resp = err.into_response();
    assert_eq!(resp.status().as_u16(), 403);
    assert!(resp.headers().get("X-Error-Code").is_some());
    let body_bytes = to_bytes(resp.into_body(), 1024*8).await.unwrap();
    let text = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(text.contains("\"code\":\"missing_role\""), "unexpected body: {}", text);
}
