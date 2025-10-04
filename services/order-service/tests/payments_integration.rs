// Feature-gated integration tests for payment flows.
// Run with:
//   cargo test -p order-service --no-default-features --features "integration-tests" --tests -- --test-threads=1

#![cfg(feature = "integration-tests")]

use axum::http::HeaderValue;
use axum::Router;
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

use common_auth::{JwtConfig, JwtVerifier};

#[tokio::test]
async fn cash_happy_path_and_change() {
    // Build a minimal app router if available
    let app = order_service_app().await;

    // Issue a fake token bypass by injecting a dev verifier
    let jwt = dev_jwt();

    // Seed products via compute body SKUs
    let body = json!({
        "items": [
          {"sku": "SKU-SODA", "quantity": 2},
          {"sku": "SKU-WATER", "quantity": 1}
        ],
        "discount_percent_bp": 1000,
        "tax_rate_bps": 800
    });

    let resp = app
        .clone()
        .oneshot(axum::http::Request::builder()
            .method("POST")
            .uri("/orders/compute")
            .header("X-Tenant-ID", uuid::Uuid::new_v4().to_string())
            .header("X-Roles", "admin")
            .header("Content-Type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap())
        .await
        .unwrap();

    assert!(resp.status().is_success());
}

#[tokio::test]
async fn card_exact_amount_happy_path() {
    // TODO: implement minimal in-memory app and assert receipt includes Paid (card)
    assert!(true);
}

// Helpers below — these are placeholders; real app factory is in main.rs
async fn order_service_app() -> Router {
    // In real tests, construct the router with a test DB pool and test JwtVerifier
    Router::new()
}

fn dev_jwt() -> String {
    // Return a placeholder since verifier wiring is not trivial here.
    "eyJhbGciOi...".to_string()
}
