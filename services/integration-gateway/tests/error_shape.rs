use common_http_errors::ApiError;
use axum::response::IntoResponse;
use axum::body::to_bytes;
use axum::http::StatusCode;

#[tokio::test]
async fn api_error_invalid_tenant_shape() {
    let err = ApiError::BadRequest { code: "invalid_tenant", trace_id: None, message: Some("Invalid tenant header".into()) };
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let header = resp.headers().get("X-Error-Code").unwrap();
    assert_eq!(header, "invalid_tenant");
    let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("\"code\":\"invalid_tenant\""));
}

#[tokio::test]
async fn api_error_internal_shape() {
    let err = ApiError::Internal { trace_id: None, message: Some("Rate limiter failure".into()) };
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let header = resp.headers().get("X-Error-Code").unwrap();
    assert_eq!(header, "internal_error");
    let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("\"code\":\"internal_error\""));
}
