use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Utc};
use common_auth::{
    ensure_role, tenant_id_from_request, AuthContext, ROLE_ADMIN, ROLE_MANAGER, ROLE_SUPER_ADMIN,
};
use rdkafka::producer::FutureRecord;
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};
use bigdecimal::BigDecimal;
use common_money::{normalize_scale, Money};
use serde_json::{json, Value};
use sqlx::{query, query_as, PgPool};
use std::{env, time::Duration};
use uuid::Uuid;

const INVENTORY_DEFAULT_THRESHOLD: i32 = 5;
#[derive(Deserialize)]
pub struct UpdateProduct {
    pub name: String,
    pub price: BigDecimal, // accept raw for backward compatibility; wrapped into Money
    pub description: String,
    pub active: bool,
    #[serde(default)]
    pub image: Option<String>,
}

fn default_product_image() -> String {
    env::var("DEFAULT_PRODUCT_IMAGE_URL")
        .unwrap_or_else(|_| "https://placehold.co/400x300?text=No+Image".to_string())
}

fn normalize_image_input(input: Option<String>) -> Option<String> {
    match input {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Some(default_product_image())
            } else {
                Some(trimmed.to_string())
            }
        }
        None => None,
    }
}

#[derive(Default, Clone)]
pub struct AuditActor {
    pub id: Option<Uuid>,
    pub name: Option<String>,
    pub email: Option<String>,
}

const PRODUCT_WRITE_ROLES: &[&str] = &[ROLE_SUPER_ADMIN, ROLE_ADMIN, ROLE_MANAGER];

fn extract_actor(headers: &HeaderMap, auth: &AuthContext) -> AuditActor {
    let mut actor = AuditActor {
        id: Some(auth.claims.subject),
        name: auth
            .claims
            .raw
            .get("name")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        email: auth
            .claims
            .raw
            .get("email")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
    };

    if let Some(value) = headers
        .get("X-User-ID")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value.trim()).ok())
    {
        actor.id = Some(value);
    }
    if let Some(value) = headers
        .get("X-User-Name")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        actor.name = Some(value);
    }
    if let Some(value) = headers
        .get("X-User-Email")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        actor.email = Some(value);
    }

    actor
}

async fn record_product_audit(
    db: &PgPool,
    actor: &AuditActor,
    product_id: Uuid,
    tenant_id: Uuid,
    action: &str,
    changes: Value,
) {
    if let Err(err) = sqlx::query(
        "INSERT INTO product_audit_log (id, product_id, tenant_id, actor_id, actor_name, actor_email, action, changes) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(product_id)
    .bind(tenant_id)
    .bind(actor.id)
    .bind(actor.name.as_deref())
    .bind(actor.email.as_deref())
    .bind(action)
    .bind(changes)
    .execute(db)
    .await
    {
        tracing::warn!(?err, product_id = %product_id, action, "Failed to write product audit log");
    }
}

fn product_to_value(product: &Product) -> Value {
    serde_json::to_value(product).unwrap_or(Value::Null)
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
    state.serialize_field("price", &self.price.inner())?;
        state.serialize_field("description", &self.description)?;
        state.serialize_field("image", &self.image)?;
        state.serialize_field("image_url", &self.image)?;
        state.serialize_field("active", &self.active)?;
        state.end()
    }
}
pub async fn update_product(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Path(product_id): axum::extract::Path<Uuid>,
    Json(upd): Json<UpdateProduct>,
) -> Result<Json<Product>, (StatusCode, String)> {
    ensure_role(&auth, PRODUCT_WRITE_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;
    let actor = extract_actor(&headers, &auth);
    let existing = query_as::<_, Product>(
    "SELECT id, tenant_id, name, price, description, image, active FROM products WHERE id = $1 AND tenant_id = $2",
    )
    .bind(product_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;
    let existing = match existing {
        Some(product) => product,
        None => return Err((StatusCode::NOT_FOUND, "Product not found".into())),
    };
    let image = normalize_image_input(upd.image);
    let product = query_as::<_, Product>(
    "UPDATE products SET name = $1, price = $2, description = $3, active = $4, image = COALESCE($5, image)\n         WHERE id = $6 AND tenant_id = $7\n         RETURNING id, tenant_id, name, price, description, image, active"
    )
    .bind(upd.name)
    .bind(normalize_scale(&upd.price))
    .bind(upd.description)
    .bind(upd.active)
    .bind(image)
    .bind(product_id)
    .bind(tenant_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;
    let changes = json!({
        "before": product_to_value(&existing),
        "after": product_to_value(&product),
    });
    record_product_audit(&state.db, &actor, product.id, tenant_id, "updated", changes).await;
    Ok(Json(product))
}

#[derive(Deserialize)]
pub struct NewProduct {
    pub name: String,
    pub price: BigDecimal, // accept raw then normalize via Money
    pub description: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct Product {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub price: Money,
    pub description: String,
    pub image: String,
    pub active: bool,
}

pub async fn create_product(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Json(new_product): Json<NewProduct>,
) -> Result<Json<Product>, (StatusCode, String)> {
    ensure_role(&auth, PRODUCT_WRITE_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;
    let actor = extract_actor(&headers, &auth);

    let product_id = Uuid::new_v4();
    let NewProduct {
        name,
        price,
        description,
        image,
    } = new_product;
    let desc = description.unwrap_or_default();
    let image = normalize_image_input(image).unwrap_or_else(default_product_image);

    let product = query_as::<_, Product>(
    "INSERT INTO products (id, tenant_id, name, price, description, active, image) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id, tenant_id, name, price, description, image, active"
    )
    .bind(product_id)
    .bind(tenant_id)
    .bind(name)
    .bind(normalize_scale(&price))
    .bind(desc)
    .bind(true)
    .bind(image)
    .fetch_one(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)))?;

    let changes = json!({
        "after": product_to_value(&product),
    });
    record_product_audit(&state.db, &actor, product.id, tenant_id, "created", changes).await;

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
    auth: AuthContext,
    headers: HeaderMap,
) -> Result<Json<Vec<Product>>, (StatusCode, String)> {
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let products = query_as::<_, Product>(
        "SELECT id, tenant_id, name, price, description, image, active FROM products WHERE tenant_id = $1",
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
    auth: AuthContext,
    headers: HeaderMap,
    Path(product_id): axum::extract::Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    ensure_role(&auth, PRODUCT_WRITE_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;
    let actor = extract_actor(&headers, &auth);

    let existing = query_as::<_, Product>(
        "SELECT id, tenant_id, name, price, description, image, active FROM products WHERE id = $1 AND tenant_id = $2",
    )
    .bind(product_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {}", e),
        )
    })?;
    let existing = match existing {
        Some(product) => product,
        None => return Err((StatusCode::NOT_FOUND, "Product not found".into())),
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

    let changes = json!({
        "before": product_to_value(&existing),
    });
    record_product_audit(
        &state.db,
        &actor,
        existing.id,
        tenant_id,
        "deleted",
        changes,
    )
    .await;

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

#[derive(Deserialize)]
pub struct ProductAuditQuery {
    limit: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ProductAuditEntry {
    id: Uuid,
    action: String,
    changes: Value,
    actor_id: Option<Uuid>,
    actor_name: Option<String>,
    actor_email: Option<String>,
    created_at: DateTime<Utc>,
}

pub async fn list_product_audit(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Path(product_id): axum::extract::Path<Uuid>,
    Query(params): Query<ProductAuditQuery>,
) -> Result<Json<Vec<ProductAuditEntry>>, (StatusCode, String)> {
    ensure_role(&auth, PRODUCT_WRITE_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let mut limit = params.limit.unwrap_or(10);
    if limit < 1 {
        limit = 1;
    } else if limit > 50 {
        limit = 50;
    }

    let entries = sqlx::query_as::<_, ProductAuditEntry>(
        "SELECT id, action, changes, actor_id, actor_name, actor_email, created_at
         FROM product_audit_log
         WHERE product_id = $1 AND tenant_id = $2
         ORDER BY created_at DESC
         LIMIT $3",
    )
    .bind(product_id)
    .bind(tenant_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {}", e),
        )
    })?;

    Ok(Json(entries))
}
