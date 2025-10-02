use common_http_errors::{ApiError};

// The common-http-errors crate exposes async test helper & macro under test context.
// Here we directly exercise IntoResponse to validate header + JSON code field.
use axum::response::IntoResponse;
use axum::body::to_bytes;
use axum::http::StatusCode;

#[tokio::test]
async fn api_error_missing_role_shape() {
    let err = ApiError::ForbiddenMissingRole { role: "customer_view", trace_id: None };
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let code_header = resp.headers().get("X-Error-Code").unwrap();
    assert_eq!(code_header, "missing_role");
    let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("\"code\":\"missing_role\""), "body={}", body);
    assert!(body.contains("customer_view"), "expected missing_role role in body: {}", body);
}

#[tokio::test]
async fn api_error_not_found_shape() {
    let err = ApiError::NotFound { code: "customer_not_found", trace_id: None };
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let header = resp.headers().get("X-Error-Code").unwrap();
    assert_eq!(header, "customer_not_found");
    let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("\"code\":\"customer_not_found\""));
}
