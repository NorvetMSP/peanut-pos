use crate::app_state::AppState;
use crate::ApiError;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
// AuthContext no longer required in handlers; SecurityCtxExtractor provides actor & tenant.
use common_security::{SecurityCtxExtractor, Role};
#[cfg(feature = "kafka")] use common_audit::AuditActor as SharedAuditActor;
#[cfg(feature = "kafka")] use rdkafka::producer::FutureRecord;
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};
use bigdecimal::BigDecimal;
use common_money::{normalize_scale, Money};
use serde_json::{json, Value};
use sqlx::{query, query_as, PgPool};
use std::env;
#[cfg(feature = "kafka")] use std::time::Duration;
use uuid::Uuid;

#[allow(dead_code)]
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
pub struct AuditActor { // local representation for legacy DB audit table
    pub id: Option<Uuid>,
    pub name: Option<String>,
    pub email: Option<String>,
}

// Legacy role constant removed; all role checks now rely on Role enum via SecurityCtxExtractor.

#[cfg(feature = "kafka")]
fn shared(actor: &AuditActor) -> SharedAuditActor {
    SharedAuditActor { id: actor.id, name: actor.name.clone(), email: actor.email.clone() }
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
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(product_id): axum::extract::Path<Uuid>,
    Json(upd): Json<UpdateProduct>,
) -> Result<Json<Product>, ApiError> {
    // Temporary dual enforcement: old roles + new context roles
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
        return Err(ApiError::ForbiddenMissingRole { role: "Manager", trace_id: sec.trace_id });
    }
    let tenant_id = sec.tenant_id;
    let actor = AuditActor { id: sec.actor.id, name: sec.actor.name.clone(), email: sec.actor.email.clone() };
    let existing = query_as::<_, Product>(
    "SELECT id, tenant_id, name, price, description, image, active FROM products WHERE id = $1 AND tenant_id = $2",
    )
    .bind(product_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e, sec.trace_id))?;
    let existing = match existing {
        Some(product) => product,
        None => return Err(ApiError::NotFound { code: "product_not_found", trace_id: sec.trace_id }),
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
    .map_err(|e| ApiError::internal(e, sec.trace_id))?;
    let changes = json!({
        "before": product_to_value(&existing),
        "after": product_to_value(&product),
    });
    record_product_audit(&state.db, &actor, product.id, tenant_id, "updated", changes.clone()).await;
    #[cfg(feature = "kafka")]
    if let Some(audit) = &state.audit_producer { let _ = audit.emit(tenant_id, shared(&actor), "product", Some(product.id), "updated", "product-service", common_audit::AuditSeverity::Info, None, changes, json!({"source":"product-service"})).await; }
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
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Json(new_product): Json<NewProduct>,
) -> Result<Json<Product>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
        return Err(ApiError::ForbiddenMissingRole { role: "Manager", trace_id: sec.trace_id });
    }
    let tenant_id = sec.tenant_id;
    let actor = AuditActor { id: sec.actor.id, name: sec.actor.name.clone(), email: sec.actor.email.clone() };

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
    .map_err(|e| ApiError::internal(e, sec.trace_id))?;

    let changes = json!({
        "after": product_to_value(&product),
    });
    record_product_audit(&state.db, &actor, product.id, tenant_id, "created", changes.clone()).await;
    #[cfg(feature = "kafka")]
    if let Some(audit) = &state.audit_producer { let _ = audit.emit(tenant_id, shared(&actor), "product", Some(product.id), "created", "product-service", common_audit::AuditSeverity::Info, None, json!({"after": product_to_value(&product)}), json!({"source":"product-service"})).await; }

    #[cfg(feature = "kafka")]
    let event = serde_json::json!({
        "product_id": product.id,
        "tenant_id": tenant_id,
        "initial_quantity": 0,
        "threshold": INVENTORY_DEFAULT_THRESHOLD,
    });
    #[cfg(feature = "kafka")]
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
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
) -> Result<Json<Vec<Product>>, ApiError> {
    let tenant_id = sec.tenant_id;

    let products = query_as::<_, Product>(
        "SELECT id, tenant_id, name, price, description, image, active FROM products WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e, sec.trace_id))?;

    Ok(Json(products))
}

pub async fn delete_product(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(product_id): axum::extract::Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
        return Err(ApiError::ForbiddenMissingRole { role: "Manager", trace_id: sec.trace_id });
    }
    let tenant_id = sec.tenant_id;
    let actor = AuditActor { id: sec.actor.id, name: sec.actor.name.clone(), email: sec.actor.email.clone() };

    let existing = query_as::<_, Product>(
        "SELECT id, tenant_id, name, price, description, image, active FROM products WHERE id = $1 AND tenant_id = $2",
    )
    .bind(product_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e, sec.trace_id))?;
    let existing = match existing {
        Some(product) => product,
        None => return Err(ApiError::NotFound { code: "product_not_found", trace_id: sec.trace_id }),
    };

    let result = query("DELETE FROM products WHERE id = $1 AND tenant_id = $2")
        .bind(product_id)
        .bind(tenant_id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e, sec.trace_id))?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound { code: "product_not_found", trace_id: sec.trace_id });
    }

    let changes = json!({
        "before": product_to_value(&existing),
    });
    record_product_audit(&state.db, &actor, existing.id, tenant_id, "deleted", changes.clone()).await;
    #[cfg(feature = "kafka")]
    if let Some(audit) = &state.audit_producer { let _ = audit.emit(tenant_id, shared(&actor), "product", Some(existing.id), "deleted", "product-service", common_audit::AuditSeverity::Info, None, json!({"before": product_to_value(&existing)}), json!({"source":"product-service"})).await; }

    #[cfg(feature = "kafka")]
    let event = serde_json::json!({
        "product_id": product_id,
        "tenant_id": tenant_id,
    });
    #[cfg(feature = "kafka")]
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
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(product_id): axum::extract::Path<Uuid>,
    Query(params): Query<ProductAuditQuery>,
) -> Result<Json<Vec<ProductAuditEntry>>, ApiError> {
    if !sec.roles.iter().any(|r| matches!(r, Role::Admin | Role::Manager)) {
        return Err(ApiError::ForbiddenMissingRole { role: "Manager", trace_id: sec.trace_id });
    }
    let tenant_id = sec.tenant_id;

    let mut limit = params.limit.unwrap_or(10);
    limit = limit.clamp(1, 50);

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
    .map_err(|e| ApiError::internal(e, sec.trace_id))?;

    Ok(Json(entries))
}
