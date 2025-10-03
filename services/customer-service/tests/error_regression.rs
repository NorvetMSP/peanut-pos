//! Minimal error regression harness for customer-service without hitting the real DB layer.
//! We re-create stub handlers that apply the same capability / role enforcement paths
//! to validate 400 (missing tenant), 403 (forbidden role), and 404 (not found) shapes.

use axum::{Router, routing::{post, get}, http::{Request, StatusCode, HeaderValue}};
use common_auth::{JwtVerifier, JwtConfig};
use std::sync::Arc;
use tower::util::ServiceExt; // provides oneshot
use uuid::Uuid;
use common_security::{SecurityCtxExtractor, ensure_capability, Capability};
use common_http_errors::ApiError;
use axum::{Json, extract::State};

#[derive(Clone)]
struct AppState { jwt_verifier: Arc<JwtVerifier> }

impl axum::extract::FromRef<AppState> for Arc<JwtVerifier> { fn from_ref(s:&AppState)->Self { s.jwt_verifier.clone() } }

fn state() -> AppState { AppState { jwt_verifier: Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud"))) } }

// Simulated request payloads
#[derive(serde::Deserialize)]
struct CreateCustomerStub {
    // Match incoming JSON keys while keeping fields unused in test logic
    #[serde(rename = "name")]
    _name: String,
    #[serde(rename = "email")]
    _email: Option<String>,
}

async fn create_customer_stub(State(_state): State<AppState>, SecurityCtxExtractor(sec): SecurityCtxExtractor, Json(_body): Json<CreateCustomerStub>) -> Result<String, ApiError> {
    if ensure_capability(&sec, Capability::CustomerWrite).is_err() {
        return Err(ApiError::ForbiddenMissingRole { role: "customer_write", trace_id: sec.trace_id });
    }
    // Always return not found to simulate a downstream condition after auth passes
    Err(ApiError::NotFound { code: "customer_not_found", trace_id: None })
}

async fn get_customer_stub(State(_state): State<AppState>, SecurityCtxExtractor(sec): SecurityCtxExtractor, axum::extract::Path(_id): axum::extract::Path<Uuid>) -> Result<String, ApiError> {
    if ensure_capability(&sec, Capability::CustomerView).is_err() {
        return Err(ApiError::ForbiddenMissingRole { role: "customer_view", trace_id: sec.trace_id });
    }
    Err(ApiError::NotFound { code: "customer_not_found", trace_id: None })
}

#[tokio::test]
async fn missing_tenant_400_create() {
    let app = Router::new().route("/customers", post(create_customer_stub)).with_state(state());
    let json_body = r#"{ "name": "Test", "email": "test@example.com" }"#;
    let req = Request::builder().uri("/customers").method("POST").header("content-type","application/json").body(axum::body::Body::from(json_body)).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST); // missing tenant header rejected by extractor
    // TA-OPS-8: Uniform 400 header emission; header must always be present now
    let code = resp.headers().get("X-Error-Code").expect("missing X-Error-Code header");
    assert_eq!(code, "missing_tenant_id");
}

#[tokio::test]
async fn forbidden_role_403_create() {
    let app = Router::new().route("/customers", post(create_customer_stub)).with_state(state());
    let json_body = r#"{ "name": "Test", "email": "test@example.com" }"#;
    let mut req = Request::builder().uri("/customers").method("POST").header("content-type","application/json").body(axum::body::Body::from(json_body)).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("support")); // support lacks write capability/fallback
    h.insert("X-User-ID", HeaderValue::from_static("22222222-2222-2222-2222-222222222222"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
}

#[tokio::test]
async fn not_found_404_get() {
    let app = Router::new().route("/customers/:id", get(get_customer_stub)).with_state(state());
    let id = Uuid::new_v4();
    let mut req = Request::builder().uri(format!("/customers/{}", id)).method("GET").body(axum::body::Body::empty()).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("admin"));
    h.insert("X-User-ID", HeaderValue::from_static("22222222-2222-2222-2222-222222222222"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "customer_not_found");
}

#[tokio::test]
async fn support_role_denied_customer_write() {
    let app = Router::new().route("/customers", post(create_customer_stub)).with_state(state());
    let json_body = r#"{ "name": "User", "email": "u@example.com" }"#;
    let mut req = Request::builder().uri("/customers").method("POST").header("content-type","application/json").body(axum::body::Body::from(json_body)).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("support"));
    h.insert("X-User-ID", HeaderValue::from_static("99999999-9999-9999-9999-999999999999"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
}

#[tokio::test]
async fn cashier_role_denied_customer_write() {
    let app = Router::new().route("/customers", post(create_customer_stub)).with_state(state());
    let json_body = r#"{ "name": "User", "email": "u@example.com" }"#;
    let mut req = Request::builder().uri("/customers").method("POST").header("content-type","application/json").body(axum::body::Body::from(json_body)).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("cashier"));
    h.insert("X-User-ID", HeaderValue::from_static("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
}

#[tokio::test]
async fn inventory_role_denied_customer_write() {
    let app = Router::new().route("/customers", post(create_customer_stub)).with_state(state());
    let json_body = r#"{ "name": "User", "email": "u@example.com" }"#;
    let mut req = Request::builder().uri("/customers").method("POST").header("content-type","application/json").body(axum::body::Body::from(json_body)).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("inventory"));
    h.insert("X-User-ID", HeaderValue::from_static("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
}

// GDPR Manage denial & allow tests
#[tokio::test]
async fn manager_denied_gdpr_manage() {
    async fn gdpr_stub(SecurityCtxExtractor(sec): SecurityCtxExtractor) -> Result<String, ApiError> {
        ensure_capability(&sec, Capability::GdprManage)
            .map_err(|_| ApiError::ForbiddenMissingRole { role: "gdpr_manage", trace_id: sec.trace_id })?;
        Ok("ok".into())
    }
    let app = Router::new().route("/gdpr/export", get(gdpr_stub)).with_state(state());
    let mut req = Request::builder().uri("/gdpr/export").method("GET").body(axum::body::Body::empty()).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("manager"));
    h.insert("X-User-ID", HeaderValue::from_static("cccccccc-cccc-cccc-cccc-cccccccccccc"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "missing_role");
}

#[tokio::test]
async fn admin_allowed_gdpr_manage() {
    async fn gdpr_stub(SecurityCtxExtractor(sec): SecurityCtxExtractor) -> Result<String, ApiError> {
        ensure_capability(&sec, Capability::GdprManage)
            .map_err(|_| ApiError::ForbiddenMissingRole { role: "gdpr_manage", trace_id: sec.trace_id })?;
        Ok("ok".into())
    }
    let app = Router::new().route("/gdpr/export", get(gdpr_stub)).with_state(state());
    let mut req = Request::builder().uri("/gdpr/export").method("GET").body(axum::body::Body::empty()).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("admin"));
    h.insert("X-User-ID", HeaderValue::from_static("dddddddd-dddd-dddd-dddd-dddddddddddd"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn superadmin_allowed_gdpr_manage() {
    async fn gdpr_stub(SecurityCtxExtractor(sec): SecurityCtxExtractor) -> Result<String, ApiError> {
        ensure_capability(&sec, Capability::GdprManage)
            .map_err(|_| ApiError::ForbiddenMissingRole { role: "gdpr_manage", trace_id: sec.trace_id })?;
        Ok("ok".into())
    }
    let app = Router::new().route("/gdpr/export", get(gdpr_stub)).with_state(state());
    let mut req = Request::builder().uri("/gdpr/export").method("GET").body(axum::body::Body::empty()).unwrap();
    let h = req.headers_mut();
    h.insert("X-Tenant-ID", HeaderValue::from_static("11111111-1111-1111-1111-111111111111"));
    h.insert("X-Roles", HeaderValue::from_static("superadmin"));
    h.insert("X-User-ID", HeaderValue::from_static("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn internal_error_500() {
    use axum::routing::get;
    use common_http_errors::ApiError;
    async fn boom() -> Result<String, ApiError> { Err(ApiError::Internal { trace_id: None, message: Some("synthetic".into()) }) }
    let app = Router::new().route("/boom", get(boom));
    let req = Request::builder().uri("/boom").method("GET").body(axum::body::Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "internal_error");
}
