use common_http_errors::ApiError;
use axum::response::IntoResponse;
use axum::http::StatusCode;
use uuid::Uuid;

#[test]
fn forbidden_missing_role_variant() {
    let err = ApiError::ForbiddenMissingRole { role: "admin", trace_id: None };
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
}

#[test]
fn forbidden_variant() {
    let err = ApiError::Forbidden { trace_id: None };
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "forbidden");
}

#[test]
fn bad_request_variant() {
    let err = ApiError::BadRequest { code: "invalid_something", trace_id: None, message: None };
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "invalid_something");
}

#[test]
fn not_found_variant() {
    let err = ApiError::NotFound { code: "missing_resource", trace_id: None };
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_resource");
}

#[test]
fn internal_variant() {
    let trace = Some(Uuid::new_v4());
    let err = ApiError::Internal { trace_id: trace, message: Some("boom".into()) };
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "internal_error");
}
