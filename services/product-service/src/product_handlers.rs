use axum::{Json, http::{HeaderMap, StatusCode}};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::AppState;
use sqlx::query_as;

#[derive(Deserialize)]
pub struct NewProduct {
    pub name: String,
    pub price: f64,
    pub description: Option<String>
}

#[derive(Serialize, Debug)]
pub struct Product {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub price: f64,
    pub description: String
}

pub async fn create_product(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(new_product): Json<NewProduct>
) -> Result<Json<Product>, (StatusCode, String)> {
    // Extract tenant ID
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => return Err((StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID header".to_string()))
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Missing X-Tenant-ID header".to_string()));
    };
    // Generate new product ID
    let product_id = Uuid::new_v4();
    // Prepare description (use empty string if not provided)
    let desc = new_product.description.unwrap_or_default();
    // Insert product into database
    let product = query_as!(
        Product,
        "INSERT INTO products (id, tenant_id, name, price, description) VALUES ($1, $2, $3, $4, $5) RETURNING id, tenant_id, name, price, description",
        product_id,
        tenant_id,
        new_product.name,
        new_product.price,
        desc
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;
    Ok(Json(product))
}

pub async fn list_products(
    State(state): State<AppState>,
    headers: HeaderMap
) -> Result<Json<Vec<Product>>, (StatusCode, String)> {
    // Extract tenant ID
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => return Err((StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID header".to_string()))
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Missing X-Tenant-ID header".to_string()));
    };
    // Query products for tenant
    let products = query_as!(
        Product,
        "SELECT id, tenant_id, name, price, description FROM products WHERE tenant_id = $1",
        tenant_id
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;
    Ok(Json(products))
}
