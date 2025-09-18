use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use rdkafka::producer::FutureRecord;
use serde::{Deserialize, Serialize};
use sqlx::query_as;
use std::time::Duration;
use uuid::Uuid;

const INVENTORY_DEFAULT_THRESHOLD: i32 = 5;

#[derive(Deserialize)]
pub struct UpdateProduct {
    pub name: String,
    pub price: f64,
    pub description: String,
    pub active: bool,
}

pub async fn update_product(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(product_id): axum::extract::Path<Uuid>,
    Json(upd): Json<UpdateProduct>,
) -> Result<Json<Product>, (StatusCode, String)> {
    // Extract tenant context
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => return Err((StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID header".into())),
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Missing X-Tenant-ID header".into()));
    };
    // Update the product
    let product = query_as::<_, Product>(
        "UPDATE products SET name = $1, price = $2, description = $3, active = $4\n         WHERE id = $5 AND tenant_id = $6\n         RETURNING id, tenant_id, name, price, description, active"
    )
    .bind(upd.name)
    .bind(upd.price)
    .bind(upd.description)
    .bind(upd.active)
    .bind(product_id)
    .bind(tenant_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;
    Ok(Json(product))
}

#[derive(Deserialize)]
pub struct NewProduct {
    pub name: String,
    pub price: f64,
    pub description: Option<String>,
}

#[derive(Serialize, Debug, sqlx::FromRow)]
pub struct Product {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub price: f64,
    pub description: String,
    pub active: bool,
}

pub async fn create_product(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(new_product): Json<NewProduct>,
) -> Result<Json<Product>, (StatusCode, String)> {
    // Extract tenant ID
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Invalid X-Tenant-ID header".to_string(),
                ))
            }
        }
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing X-Tenant-ID header".to_string(),
        ));
    };
    // Generate new product ID
    let product_id = Uuid::new_v4();
    // Extract payload fields and normalize description
    let NewProduct {
        name,
        price,
        description,
    } = new_product;
    let desc = description.unwrap_or_default();
    // Insert product into database
    let product = query_as::<_, Product>(
        "INSERT INTO products (id, tenant_id, name, price, description, active) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, tenant_id, name, price, description, active"
    )
    .bind(product_id)
    .bind(tenant_id)
    .bind(name)
    .bind(price)
    .bind(desc)
    .bind(true) // new products default to active
    .fetch_one(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;

    let event = serde_json::json!({
        "product_id": product.id,
        "tenant_id": tenant_id,
        "initial_quantity": 0,
        "threshold": INVENTORY_DEFAULT_THRESHOLD,
    });
    if let Err(err) = state
        .kafka_producer
        .send(
            FutureRecord::to("product.created")
                .payload(&event.to_string())
                .key(&tenant_id.to_string()),
            Duration::from_secs(0),
        )
        .await
    {
        tracing::error!("Failed to publish product.created event: {:?}", err);
    }

    Ok(Json(product))
}

pub async fn list_products(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Product>>, (StatusCode, String)> {
    // Extract tenant ID
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Invalid X-Tenant-ID header".to_string(),
                ))
            }
        }
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing X-Tenant-ID header".to_string(),
        ));
    };
    // Query products for tenant
    let products = query_as::<_, Product>(
        "SELECT id, tenant_id, name, price, description, active FROM products WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {}", e),
        )
    })?;
    Ok(Json(products))
}
