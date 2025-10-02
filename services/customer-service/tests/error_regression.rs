//! Minimal error regression harness for customer-service without hitting the real DB layer.
//! We re-create stub handlers that apply the same capability / role enforcement paths
//! to validate 400 (missing tenant), 403 (forbidden role), and 404 (not found) shapes.

use axum::{Router, routing::{post, get}, http::{Request, StatusCode, HeaderValue}};
use common_auth::{JwtVerifier, JwtConfig};
use std::sync::Arc;
use tower::util::ServiceExt; // provides oneshot
use uuid::Uuid;
use common_security::{SecurityCtxExtractor, ensure_capability, Capability, roles::{ensure_any_role, Role}};
use common_http_errors::ApiError;
use axum::{Json, extract::State};

#[derive(Clone)]
struct AppState { jwt_verifier: Arc<JwtVerifier> }

impl axum::extract::FromRef<AppState> for Arc<JwtVerifier> { fn from_ref(s:&AppState)->Self { s.jwt_verifier.clone() } }

fn state() -> AppState { AppState { jwt_verifier: Arc::new(JwtVerifier::new(JwtConfig::new("issuer","aud"))) } }

// Simulated request payloads
#[derive(serde::Deserialize)]
struct CreateCustomerStub { name: String, email: Option<String> }

const CUSTOMER_WRITE_ROLES: &[Role] = &[Role::SuperAdmin, Role::Admin, Role::Manager, Role::Inventory, Role::Cashier];
const CUSTOMER_VIEW_ROLES: &[Role]  = &[Role::SuperAdmin, Role::Admin, Role::Manager, Role::Inventory, Role::Cashier];

async fn create_customer_stub(State(_state): State<AppState>, SecurityCtxExtractor(sec): SecurityCtxExtractor, Json(_body): Json<CreateCustomerStub>) -> Result<String, ApiError> {
    if let Err(_) = ensure_capability(&sec, Capability::CustomerWrite) {
        if ensure_any_role(&sec, CUSTOMER_WRITE_ROLES).is_err() {
            return Err(ApiError::ForbiddenMissingRole { role: "customer_write", trace_id: sec.trace_id });
        }
    }
    // Always return not found to simulate a downstream condition after auth passes
    Err(ApiError::NotFound { code: "customer_not_found", trace_id: None })
}

async fn get_customer_stub(State(_state): State<AppState>, SecurityCtxExtractor(sec): SecurityCtxExtractor, axum::extract::Path(_id): axum::extract::Path<Uuid>) -> Result<String, ApiError> {
    if let Err(_) = ensure_capability(&sec, Capability::CustomerView) {
        if ensure_any_role(&sec, CUSTOMER_VIEW_ROLES).is_err() {
            return Err(ApiError::ForbiddenMissingRole { role: "customer_view", trace_id: sec.trace_id });
        }
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
}
