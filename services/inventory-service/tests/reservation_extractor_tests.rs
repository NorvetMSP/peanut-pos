use inventory_service::create_reservation;
mod test_utils;
use test_utils::lazy_app_state;
use axum::{Router, routing::post};
use axum::http::{Request, HeaderValue, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;
use serde_json::json;

#[tokio::test]
async fn create_reservation_rejects_empty_items() {
    let state = lazy_app_state();

    let app = Router::new().route("/inventory/reservations", post(create_reservation)).with_state(state);

    let tenant_id = Uuid::new_v4();
    let body = json!({
        "order_id": Uuid::new_v4(),
        "items": []
    }).to_string();

    let mut req = Request::builder().uri("/inventory/reservations").method("POST")
        .header("content-type","application/json")
        .body(axum::body::Body::from(body)).unwrap();
    let headers = req.headers_mut();
    headers.insert("X-Tenant-ID", HeaderValue::from_str(&tenant_id.to_string()).unwrap());
    headers.insert("X-Roles", HeaderValue::from_static("admin"));
    headers.insert("X-User-ID", HeaderValue::from_str(&Uuid::new_v4().to_string()).unwrap());

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "empty items should be rejected");
    assert_eq!(resp.headers().get("X-Error-Code").unwrap(), "empty_reservation");
}
