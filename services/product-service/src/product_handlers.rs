use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use rdkafka::producer::FutureRecord;
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};
use sqlx::{query, query_as};
use std::time::Duration;
use uuid::Uuid;

const INVENTORY_DEFAULT_THRESHOLD: i32 = 5;
const DEFAULT_PRODUCT_IMAGE: &str = "https://placehold.co/400x300?text=No+Image";

#[derive(Deserialize)]
pub struct UpdateProduct {
    pub name: String,
    pub price: f64,
    pub description: String,
    pub active: bool,
    #[serde(default)]
    pub image: Option<String>,
}

fn normalize_image_input(input: Option<String>) -> Option<String> {
    match input {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Some(DEFAULT_PRODUCT_IMAGE.to_string())
            } else {
                Some(trimmed.to_string())
            }
        }
        None => None,
    }
}

impl Serialize for Product {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Product", 8)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("tenant_id", &self.tenant_id)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("price", &self.price)?;
        state.serialize_field("description", &self.description)?;
        state.serialize_field("image", &self.image)?;
        state.serialize_field("image_url", &self.image)?;
        state.serialize_field("active", &self.active)?;
        state.end()
    }
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
    let image = normalize_image_input(upd.image);
    let product = query_as::<_, Product>(
        "UPDATE products SET name = $1, price = $2, description = $3, active = $4, image = COALESCE($5, image)\n         WHERE id = $6 AND tenant_id = $7\n         RETURNING id, tenant_id, name, price::FLOAT8 as price, description, image, active"
    )
    .bind(upd.name)
    .bind(upd.price)
    .bind(upd.description)
    .bind(upd.active)
    .bind(image)
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
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct Product {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub price: f64,
    pub description: String,
    pub image: String,
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
        image,
    } = new_product;
    let desc = description.unwrap_or_default();
    let image = normalize_image_input(image).unwrap_or_else(|| DEFAULT_PRODUCT_IMAGE.to_string());
    // Insert product into database
    let product = query_as::<_, Product>(
        "INSERT INTO products (id, tenant_id, name, price, description, active, image) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id, tenant_id, name, price::FLOAT8 as price, description, image, active"
    )
    .bind(product_id)
    .bind(tenant_id)
    .bind(name)
    .bind(price)
    .bind(desc)
    .bind(true) // new products default to active
    .bind(image)
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
        "SELECT id, tenant_id, name, price::FLOAT8 as price, description, image, active FROM products WHERE tenant_id = $1",
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

pub async fn delete_product(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(product_id): axum::extract::Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let tenant_id = if let Some(hdr) = headers.get("X-Tenant-ID") {
        match hdr.to_str().ok().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => id,
            None => return Err((StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID header".into())),
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Missing X-Tenant-ID header".into()));
    };

    let result = query("DELETE FROM products WHERE id = $1 AND tenant_id = $2")
        .bind(product_id)
        .bind(tenant_id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            )
        })?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Product not found".into()));
    }

    let event = serde_json::json!({
        "product_id": product_id,
        "tenant_id": tenant_id,
    });
    if let Err(err) = state
        .kafka_producer
        .send(
            FutureRecord::to("product.deleted")
                .payload(&event.to_string())
                .key(&tenant_id.to_string()),
            Duration::from_secs(0),
        )
        .await
    {
        tracing::error!("Failed to publish product.deleted event: {:?}", err);
    }

    Ok(StatusCode::NO_CONTENT)
}
