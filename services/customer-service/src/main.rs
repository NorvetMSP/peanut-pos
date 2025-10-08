use anyhow::{anyhow, Context};
use axum::{
    extract::{FromRef, Path, State},
    http::{
        header::{ACCEPT, CONTENT_TYPE},
    HeaderName, HeaderValue, Method, StatusCode,
    },
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use common_auth::{
    JwtConfig, JwtVerifier,
};
use common_security::{SecurityCtxExtractor, ensure_capability, Capability};
#[cfg(test)] use common_security::roles::Role;
use common_http_errors::{ApiError, ApiResult};
use common_crypto::{decrypt_field, deterministic_hash, encrypt_field, CryptoError, MasterKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Executor, FromRow, PgPool};
use std::{
    collections::HashMap,
    env,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::net::TcpListener;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn, error};
use once_cell::sync::Lazy;
use prometheus::{IntCounterVec, Opts, TextEncoder, Encoder};
use common_money::log_rounding_mode_once;
use uuid::Uuid;

static HTTP_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new(
            "http_errors_total",
            "Count of HTTP error responses emitted (status >= 400)",
        ),
        &["service", "code", "status"],
    )
    .expect("http_errors_total");
    let _ = prometheus::default_registry().register(Box::new(c.clone()));
    c
});

async fn track_http_errors(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::response::Response> {
    let resp = next.run(req).await;
    let status = resp.status();
    if status.as_u16() >= 400 {
        let code = resp
            .headers()
            .get("X-Error-Code")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");
        HTTP_ERRORS_TOTAL
            .with_label_values(&["customer-service", code, status.as_str()])
            .inc();
    }
    Ok(resp)
}

async fn render_metrics() -> Result<String, StatusCode> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    if encoder.encode(&metric_families, &mut buffer).is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    String::from_utf8(buffer).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

// Legacy CUSTOMER_*_ROLES arrays retained only for tests until fallback fully removed.
mod handlers;
use handlers::{create_customer, get_customer, update_customer, search_customers};
// GDPR management now gated by Capability::GdprManage (was CustomerWrite pre-refinement TA-POL-5)
const GDPR_DELETED_NAME: &str = "[deleted]";

#[derive(Clone)]
struct AppState {
    db: PgPool,
    jwt_verifier: Arc<JwtVerifier>,
    master_key: Arc<MasterKey>,
}

// ApiResult now comes from common-http-errors (Result<T, ApiError>)

impl FromRef<AppState> for Arc<JwtVerifier> {
    fn from_ref(state: &AppState) -> Self {
        state.jwt_verifier.clone()
    }
}

#[derive(Deserialize)]
struct NewCustomer {
    name: String,
    email: Option<String>,
    phone: Option<String>,
}

#[derive(Deserialize)]
struct UpdateCustomerRequest {
    name: Option<String>,
    email: Option<Option<String>>,
    phone: Option<Option<String>>,
}

#[derive(Serialize)]
struct Customer {
    id: Uuid,
    tenant_id: Uuid,
    name: String,
    email: Option<String>,
    phone: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct GdprExportResponse {
    export_id: Uuid,
    customer: Customer,
}

#[derive(Serialize)]
struct GdprDeleteResponse {
    tombstone_id: Uuid,
    status: String,
}

#[derive(FromRow)]
struct CustomerRow {
    id: Uuid,
    tenant_id: Uuid,
    name: String,
    email: Option<String>,
    phone: Option<String>,
    email_encrypted: Option<Vec<u8>>,
    phone_encrypted: Option<Vec<u8>>,
    pii_key_version: Option<i32>,
    created_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct TenantKeyRow {
    key_version: i32,
    encrypted_key: Vec<u8>,
}

struct TenantDek {
    version: i32,
    key: [u8; 32],
}

struct TenantKeyCache<'a> {
    db: &'a PgPool,
    master: Arc<MasterKey>,
    tenant_id: Uuid,
    cache: HashMap<i32, [u8; 32]>,
}

impl<'a> TenantKeyCache<'a> {
    fn new(state: &'a AppState, tenant_id: Uuid) -> Self {
        Self {
            db: &state.db,
            master: state.master_key.clone(),
            tenant_id,
            cache: HashMap::new(),
        }
    }

    fn prime(&mut self, dek: &TenantDek) {
        self.cache.insert(dek.version, dek.key);
    }

    async fn active(&mut self) -> ApiResult<TenantDek> {
        let dek = load_tenant_dek(self.db, self.master.as_ref(), self.tenant_id, None).await?;
        self.cache.entry(dek.version).or_insert(dek.key);
        Ok(dek)
    }

    async fn by_version(&mut self, version: i32) -> ApiResult<[u8; 32]> {
        if let Some(existing) = self.cache.get(&version) {
            return Ok(*existing);
        }
        let dek =
            load_tenant_dek(self.db, self.master.as_ref(), self.tenant_id, Some(version)).await?;
        self.cache.insert(dek.version, dek.key);
        Ok(dek.key)
    }
}

#[derive(Deserialize, Default)]
struct SearchParams {
    q: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    log_rounding_mode_once();

    let database_url = env::var("DATABASE_URL")?;
    let db_pool = PgPool::connect(&database_url).await?;

    let master_key_raw =
        env::var("CUSTOMER_MASTER_KEY").context("CUSTOMER_MASTER_KEY must be set")?;
    let master_key = MasterKey::from_base64(&master_key_raw)
        .map_err(|err| anyhow!("failed to decode CUSTOMER_MASTER_KEY: {err}"))?;

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    let state = AppState {
        db: db_pool,
        jwt_verifier,
        master_key: Arc::new(master_key),
    };

    let allowed_origins = [
        "http://localhost:3000",
        "http://localhost:3001",
        "http://localhost:5173",
    ];

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(
            allowed_origins
                .iter()
                .filter_map(|origin| origin.parse::<HeaderValue>().ok())
                .collect::<Vec<_>>(),
        ))
        .allow_methods([Method::GET, Method::POST, Method::PUT])
        .allow_headers([
            ACCEPT,
            CONTENT_TYPE,
            HeaderName::from_static("authorization"),
            HeaderName::from_static("x-tenant-id"),
        ]);

    let app = Router::new()
        .route("/customers", post(create_customer).get(search_customers))
        .route("/customers/:id", get(get_customer).put(update_customer))
        .route("/customers/:id/gdpr/export", post(gdpr_export_customer))
        .route("/customers/:id/gdpr/delete", post(gdpr_delete_customer))
        .route("/healthz", get(|| async { "ok" }))
        .route("/internal/metrics", get(render_metrics))
        .route("/metrics", get(render_metrics))
        .with_state(state)
        .layer(axum::middleware::from_fn(track_http_errors))
        .layer(cors);

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8089);
    let ip: IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));

    println!("starting customer-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

// Implementation moved for reuse (public via crate::create_customer_impl)
pub(crate) async fn create_customer_impl(
    state: AppState,
    sec: common_security::context::SecurityContext,
    new_cust: NewCustomer,
) -> ApiResult<Json<Customer>> {
    ensure_capability(&sec, Capability::GdprManage)
        .map_err(|_| ApiError::ForbiddenMissingRole { role: "customer_write", trace_id: sec.trace_id })?;
    let tenant_id = sec.tenant_id;
    let customer_id = Uuid::new_v4();

    let mut key_cache = TenantKeyCache::new(&state, tenant_id);
    let active_key = key_cache.active().await?;
    key_cache.prime(&active_key);

    let NewCustomer { name, email, phone } = new_cust;
    let name = name.trim().to_string();
    let email = sanitize_optional(email);
    let phone = sanitize_optional(phone);

    let email_encrypted = match &email {
        Some(value) => Some(encrypt_field(&active_key.key, value.as_bytes()).map_err(crypto_err)?),
        None => None,
    };
    let email_hash = match &email {
        Some(value) => {
            let normalized = normalize_email(value);
            if normalized.is_empty() {
                None
            } else {
                Some(
                    deterministic_hash(&active_key.key, normalized.as_bytes())
                        .map_err(crypto_err)?,
                )
            }
        }
        None => None,
    };

    let phone_encrypted = match &phone {
        Some(value) => Some(encrypt_field(&active_key.key, value.as_bytes()).map_err(crypto_err)?),
        None => None,
    };
    let phone_hash = match &phone {
        Some(value) => {
            let normalized = normalize_phone(value);
            if normalized.is_empty() {
                None
            } else {
                Some(
                    deterministic_hash(&active_key.key, normalized.as_bytes())
                        .map_err(crypto_err)?,
                )
            }
        }
        None => None,
    };

    let row = sqlx::query_as::<_, CustomerRow>(
        "INSERT INTO customers (
            id,
            tenant_id,
            name,
            email,
            phone,
            email_encrypted,
            phone_encrypted,
            email_hash,
            phone_hash,
            pii_key_version,
            pii_encrypted_at,
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11
        )
        RETURNING
            id,
            tenant_id,
            name,
            email,
            phone,
            email_encrypted,
            phone_encrypted,
            pii_key_version,
            created_at",
    )
    .bind(customer_id)
    .bind(tenant_id)
    .bind(&name)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(email_encrypted.as_deref())
    .bind(phone_encrypted.as_deref())
    .bind(email_hash.as_deref())
    .bind(phone_hash.as_deref())
    .bind(Some(active_key.version))
    .bind(Some(Utc::now()))
    .fetch_one(&state.db)
    .await
    .map_err(db_internal)?;

    let customer = hydrate_customer_row(row, &mut key_cache).await?;
    Ok(Json(customer))
}

pub(crate) async fn search_customers_impl(
    state: AppState,
    sec: common_security::context::SecurityContext,
    params: SearchParams,
) -> ApiResult<Json<Vec<Customer>>> {
    ensure_capability(&sec, Capability::CustomerView)
        .map_err(|_| ApiError::ForbiddenMissingRole { role: "customer_view", trace_id: sec.trace_id })?;
    let tenant_id = sec.tenant_id;

    let mut key_cache = TenantKeyCache::new(&state, tenant_id);
    let active_key = key_cache.active().await?;
    key_cache.prime(&active_key);

    let search_term = params.q.unwrap_or_default();
    let trimmed = search_term.trim();
    let pattern = if trimmed.is_empty() {
        "%".to_string()
    } else {
        format!("%{}%", trimmed)
    };

    let email_hash = if trimmed.is_empty() {
        None
    } else {
        let normalized = normalize_email(trimmed);
        if normalized.is_empty() {
            None
        } else {
            Some(deterministic_hash(&active_key.key, normalized.as_bytes()).map_err(crypto_err)?)
        }
    };

    let phone_hash = if trimmed.is_empty() {
        None
    } else {
        let normalized = normalize_phone(trimmed);
        if normalized.is_empty() {
            None
        } else {
            Some(deterministic_hash(&active_key.key, normalized.as_bytes()).map_err(crypto_err)?)
        }
    };

    let rows = sqlx::query_as::<_, CustomerRow>(
        "SELECT
            id,
            tenant_id,
            name,
            email,
            phone,
            email_encrypted,
            phone_encrypted,
            pii_key_version,
            created_at
        FROM customers
        WHERE tenant_id = $1
          AND (
                name ILIKE $2
             OR ($3::bytea IS NOT NULL AND email_hash = $3)
             OR ($4::bytea IS NOT NULL AND phone_hash = $4)
          )
        ORDER BY created_at DESC
        LIMIT 20",
    )
    .bind(tenant_id)
    .bind(&pattern)
    .bind(email_hash.as_deref())
    .bind(phone_hash.as_deref())
    .fetch_all(&state.db)
    .await
    .map_err(db_internal)?;

    let customers = hydrate_customer_rows(rows, &mut key_cache).await?;
    Ok(Json(customers))
}

pub(crate) async fn get_customer_impl(
    state: AppState,
    sec: common_security::context::SecurityContext,
    customer_id: Uuid,
) -> ApiResult<Json<Customer>> {
    ensure_capability(&sec, Capability::CustomerView)
        .map_err(|_| ApiError::ForbiddenMissingRole { role: "customer_view", trace_id: sec.trace_id })?;
    let tenant_id = sec.tenant_id;

    let mut key_cache = TenantKeyCache::new(&state, tenant_id);

    let row = sqlx::query_as::<_, CustomerRow>(
        "SELECT
            id,
            tenant_id,
            name,
            email,
            phone,
            email_encrypted,
            phone_encrypted,
            pii_key_version,
            created_at
        FROM customers
        WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant_id)
    .bind(customer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(db_internal)?
    .ok_or(ApiError::NotFound { code: "customer_not_found", trace_id: None })?;

    let customer = hydrate_customer_row(row, &mut key_cache).await?;
    Ok(Json(customer))
}

pub(crate) async fn update_customer_impl(
    state: AppState,
    sec: common_security::context::SecurityContext,
    customer_id: Uuid,
    payload: UpdateCustomerRequest,
) -> ApiResult<Json<Customer>> {
    ensure_capability(&sec, Capability::CustomerWrite)
        .map_err(|_| ApiError::ForbiddenMissingRole { role: "customer_write", trace_id: sec.trace_id })?;
    let tenant_id = sec.tenant_id;
    let mut key_cache = TenantKeyCache::new(&state, tenant_id);

    let existing = sqlx::query_as::<_, CustomerRow>(
        "SELECT id, tenant_id, name, email, phone, email_encrypted, phone_encrypted, pii_key_version, created_at FROM customers WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant_id)
    .bind(customer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(db_internal)?
    .ok_or(ApiError::NotFound { code: "customer_not_found", trace_id: None })?;

    let UpdateCustomerRequest { name, email, phone } = payload;

    let mut existing_email = existing.email.clone();
    if existing_email.is_none() {
        existing_email = decrypt_optional_field(
            existing.email_encrypted.clone(),
            existing.pii_key_version,
            &mut key_cache,
        )
        .await?;
    }

    let mut existing_phone = existing.phone.clone();
    if existing_phone.is_none() {
        existing_phone = decrypt_optional_field(
            existing.phone_encrypted.clone(),
            existing.pii_key_version,
            &mut key_cache,
        )
        .await?;
    }

    let mut final_name = existing.name.clone();
    let mut name_changed = false;
    if let Some(candidate) = name.as_ref() {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            return Err(ApiError::BadRequest { code: "invalid_name", trace_id: None, message: Some("Name must not be empty".into()) });
        }
        if trimmed != existing.name {
            final_name = trimmed.to_string();
            name_changed = true;
        }
    }

    let final_email = match email {
        Some(value) => sanitize_optional(value),
        None => existing_email.clone(),
    };
    let final_phone = match phone {
        Some(value) => sanitize_optional(value),
        None => existing_phone.clone(),
    };

    let email_changed = final_email != existing_email;
    let phone_changed = final_phone != existing_phone;

    if !name_changed && !email_changed && !phone_changed {
        let row = CustomerRow {
            id: existing.id,
            tenant_id: existing.tenant_id,
            name: existing.name.clone(),
            email: existing.email.clone(),
            phone: existing.phone.clone(),
            email_encrypted: existing.email_encrypted.clone(),
            phone_encrypted: existing.phone_encrypted.clone(),
            pii_key_version: existing.pii_key_version,
            created_at: existing.created_at,
        };
        let customer = hydrate_customer_row(row, &mut key_cache).await?;
        return Ok(Json(customer));
    }

    let (
        email_encrypted_param,
        email_hash_param,
        phone_encrypted_param,
        phone_hash_param,
        pii_key_version_param,
        pii_encrypted_at_param,
    ) = if final_email.is_some() || final_phone.is_some() {
        let active_key = key_cache.active().await?;
        let key_bytes = active_key.key;

        let (enc_email, hash_email) = match final_email.as_ref() {
            Some(value) => {
                let encrypted = encrypt_field(&key_bytes, value.as_bytes()).map_err(crypto_err)?;
                let normalized = normalize_email(value);
                let hash = if normalized.is_empty() {
                    None
                } else {
                    Some(
                        deterministic_hash(&key_bytes, normalized.as_bytes())
                            .map_err(crypto_err)?,
                    )
                };
                (Some(encrypted), hash)
            }
            None => (None, None),
        };

        let (enc_phone, hash_phone) = match final_phone.as_ref() {
            Some(value) => {
                let encrypted = encrypt_field(&key_bytes, value.as_bytes()).map_err(crypto_err)?;
                let normalized = normalize_phone(value);
                let hash = if normalized.is_empty() {
                    None
                } else {
                    Some(
                        deterministic_hash(&key_bytes, normalized.as_bytes())
                            .map_err(crypto_err)?,
                    )
                };
                (Some(encrypted), hash)
            }
            None => (None, None),
        };

        (
            enc_email,
            hash_email,
            enc_phone,
            hash_phone,
            Some(active_key.version),
            Some(Utc::now()),
        )
    } else {
        (None, None, None, None, None, None)
    };

    let row = sqlx::query_as::<_, CustomerRow>(
        "UPDATE customers
        SET name = $1,
            email = NULL,
            phone = NULL,
            email_encrypted = $2,
            phone_encrypted = $3,
            email_hash = $4,
            phone_hash = $5,
            pii_key_version = $6,
            pii_encrypted_at = $7
        WHERE tenant_id = $8 AND id = $9
        RETURNING id, tenant_id, name, email, phone, email_encrypted, phone_encrypted, pii_key_version, created_at",
    )
    .bind(&final_name)
    .bind(email_encrypted_param.as_deref())
    .bind(phone_encrypted_param.as_deref())
    .bind(email_hash_param.as_deref())
    .bind(phone_hash_param.as_deref())
    .bind(pii_key_version_param)
    .bind(pii_encrypted_at_param)
    .bind(tenant_id)
    .bind(customer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(db_internal)?
    .ok_or(ApiError::NotFound { code: "customer_not_found", trace_id: None })?;

    let customer = hydrate_customer_row(row, &mut key_cache).await?;
    Ok(Json(customer))
}

async fn gdpr_export_customer(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(customer_id): Path<Uuid>,
) -> ApiResult<Json<GdprExportResponse>> {
    ensure_capability(&sec, Capability::CustomerWrite)
        .map_err(|_| ApiError::ForbiddenMissingRole { role: "customer_write", trace_id: sec.trace_id })?;
    let tenant_id = sec.tenant_id;

    let mut key_cache = TenantKeyCache::new(&state, tenant_id);

    let row = sqlx::query_as::<_, CustomerRow>(
        "SELECT
            id,
            tenant_id,
            name,
            email,
            phone,
            email_encrypted,
            phone_encrypted,
            pii_key_version,
            created_at
        FROM customers
        WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant_id)
    .bind(customer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(db_internal)?
    .ok_or(ApiError::NotFound { code: "customer_not_found", trace_id: None })?;

    let customer = hydrate_customer_row(row, &mut key_cache).await?;
    let metadata = json!({
        "request_type": "export",
        "had_email": customer.email.is_some(),
        "had_phone": customer.phone.is_some(),
        "requested_at": Utc::now(),
    });

    let export_id = insert_gdpr_tombstone(
        &state.db,
        tenant_id,
        Some(customer_id),
        "export",
        "completed",
    sec.actor.id,
        metadata,
    )
    .await
    .map_err(db_internal)?;

    info!(tenant_id = %tenant_id, customer_id = %customer_id, export_id = %export_id, "GDPR export completed");
    Ok(Json(GdprExportResponse {
        export_id,
        customer,
    }))
}

async fn gdpr_delete_customer(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(customer_id): Path<Uuid>,
) -> ApiResult<Json<GdprDeleteResponse>> {
    ensure_capability(&sec, Capability::CustomerWrite)
        .map_err(|_| ApiError::ForbiddenMissingRole { role: "customer_write", trace_id: sec.trace_id })?;
    let tenant_id = sec.tenant_id;

    let row = sqlx::query_as::<_, CustomerRow>(
        "SELECT
            id,
            tenant_id,
            name,
            email,
            phone,
            email_encrypted,
            phone_encrypted,
            pii_key_version,
            created_at
        FROM customers
        WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant_id)
    .bind(customer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(db_internal)?
    .ok_or(ApiError::NotFound { code: "customer_not_found", trace_id: None })?;

    let had_email = row.email.is_some() || row.email_encrypted.is_some();
    let had_phone = row.phone.is_some() || row.phone_encrypted.is_some();

    let mut tx = state.db.begin().await.map_err(db_internal)?;
    let result = sqlx::query(
        "UPDATE customers
         SET name = $1,
             email = NULL,
             phone = NULL,
             email_encrypted = NULL,
             phone_encrypted = NULL,
             email_hash = NULL,
             phone_hash = NULL,
             pii_key_version = NULL,
             pii_encrypted_at = NULL
         WHERE tenant_id = $2 AND id = $3",
    )
    .bind(GDPR_DELETED_NAME)
    .bind(tenant_id)
    .bind(customer_id)
    .execute(&mut *tx)
    .await
    .map_err(db_internal)?;

    if result.rows_affected() == 0 {
    tx.rollback().await.map_err(db_internal)?;
        return Err(ApiError::NotFound { code: "customer_not_found", trace_id: None });
    }

    let metadata = json!({
        "request_type": "delete",
        "had_email": had_email,
        "had_phone": had_phone,
        "requested_at": Utc::now(),
    });

    let tombstone_id = insert_gdpr_tombstone(
        &mut *tx,
        tenant_id,
        Some(customer_id),
        "delete",
        "completed",
    sec.actor.id,
        metadata,
    )
    .await
    .map_err(db_internal)?;

    tx.commit().await.map_err(db_internal)?;
    info!(tenant_id = %tenant_id, customer_id = %customer_id, tombstone_id = %tombstone_id, "GDPR delete completed");
    Ok(Json(GdprDeleteResponse {
        tombstone_id,
        status: "deleted".into(),
    }))
}

async fn hydrate_customer_rows(
    rows: Vec<CustomerRow>,
    key_cache: &mut TenantKeyCache<'_>,
) -> ApiResult<Vec<Customer>> {
    let mut customers = Vec::with_capacity(rows.len());
    for row in rows {
        customers.push(hydrate_customer_row(row, key_cache).await?);
    }
    Ok(customers)
}

async fn hydrate_customer_row(
    row: CustomerRow,
    key_cache: &mut TenantKeyCache<'_>,
) -> ApiResult<Customer> {
    let CustomerRow {
        id,
        tenant_id,
        name,
        email,
        phone,
        email_encrypted,
        phone_encrypted,
        pii_key_version,
        created_at,
        ..
    } = row;

    let key_version = pii_key_version;
    let decrypted_email = decrypt_optional_field(email_encrypted, key_version, key_cache).await?;
    let email = decrypted_email.or(email);

    let decrypted_phone = decrypt_optional_field(phone_encrypted, key_version, key_cache).await?;
    let phone = decrypted_phone.or(phone);

    Ok(Customer {
        id,
        tenant_id,
        name,
        email,
        phone,
        created_at,
    })
}

async fn decrypt_optional_field(
    encrypted: Option<Vec<u8>>,
    key_version: Option<i32>,
    key_cache: &mut TenantKeyCache<'_>,
) -> ApiResult<Option<String>> {
    let Some(ciphertext) = encrypted else {
        return Ok(None);
    };
    let version = key_version.ok_or(ApiError::Internal { trace_id: None, message: Some("Encrypted value missing key version".into()) })?;
    let key = key_cache.by_version(version).await?;
    let plaintext = decrypt_field(&key, &ciphertext).map_err(crypto_err)?;
    String::from_utf8(plaintext).map(Some).map_err(|_| ApiError::Internal { trace_id: None, message: Some("Decrypted value was not valid UTF-8".into()) })
}

async fn load_tenant_dek(
    db: &PgPool,
    master: &MasterKey,
    tenant_id: Uuid,
    version: Option<i32>,
) -> ApiResult<TenantDek> {
    let row = if let Some(version) = version {
        sqlx::query_as::<_, TenantKeyRow>(
            "SELECT key_version, encrypted_key
             FROM tenant_data_keys
             WHERE tenant_id = $1 AND key_version = $2
             LIMIT 1",
        )
        .bind(tenant_id)
        .bind(version)
        .fetch_optional(db)
        .await
    .map_err(db_internal)?
    } else {
        sqlx::query_as::<_, TenantKeyRow>(
            "SELECT key_version, encrypted_key
             FROM tenant_data_keys
             WHERE tenant_id = $1 AND active = TRUE
             ORDER BY key_version DESC
             LIMIT 1",
        )
        .bind(tenant_id)
        .fetch_optional(db)
        .await
    .map_err(db_internal)?
    };

    let row = row.ok_or_else(|| {
        let scope = match version {
            Some(v) => format!("version {v}"),
            None => "active".to_string(),
        };
        warn!(tenant_id = %tenant_id, scope = %scope, "Missing tenant data key");
        ApiError::Internal { trace_id: None, message: Some(format!("No {scope} tenant data key found for tenant {tenant_id}")) }
    })?;

    let TenantKeyRow {
        key_version,
        encrypted_key,
    } = row;

    let key = master
        .decrypt_tenant_dek(&encrypted_key)
        .map_err(|err| {
            error!(tenant_id = %tenant_id, version = key_version, error = ?err, "Failed to decrypt tenant data key");
            ApiError::Internal { trace_id: None, message: Some("Failed to decrypt tenant data key".into()) }
        })?;
    Ok(TenantDek {
        version: key_version,
        key,
    })
}

async fn insert_gdpr_tombstone<'a, E>(
    executor: E,
    tenant_id: Uuid,
    customer_id: Option<Uuid>,
    request_type: &str,
    status: &str,
    requested_by: Option<Uuid>,
    metadata: Value,
) -> Result<Uuid, sqlx::Error>
where
    E: Executor<'a, Database = sqlx::Postgres>,
{
    let id = Uuid::new_v4();
    let processed_at = if status == "completed" {
        Some(Utc::now())
    } else {
        None
    };
    sqlx::query(
        "INSERT INTO gdpr_tombstones (id, tenant_id, customer_id, request_type, status, requested_by, metadata, processed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(id)
    .bind(tenant_id)
    .bind(customer_id)
    .bind(request_type)
    .bind(status)
    .bind(requested_by)
    .bind(metadata)
    .bind(processed_at)
    .execute(executor)
    .await?;
    Ok(id)
}
fn sanitize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn normalize_email(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_phone(value: &str) -> String {
    value.chars().filter(|c| c.is_ascii_digit()).collect()
}

fn crypto_err(err: CryptoError) -> ApiError {
    error!(error = ?err, "Crypto operation failed");
    ApiError::Internal { trace_id: None, message: Some("Crypto operation failed".into()) }
}

fn db_internal(err: sqlx::Error) -> ApiError {
    ApiError::Internal { trace_id: None, message: Some(format!("DB error: {}", err)) }
}


async fn build_jwt_verifier_from_env() -> anyhow::Result<Arc<JwtVerifier>> {
    let issuer = env::var("JWT_ISSUER").context("JWT_ISSUER must be set")?;
    let audience = env::var("JWT_AUDIENCE").context("JWT_AUDIENCE must be set")?;

    let mut config = JwtConfig::new(issuer, audience);
    if let Ok(value) = env::var("JWT_LEEWAY_SECONDS") {
        if let Ok(leeway) = value.parse::<u32>() {
            config = config.with_leeway(leeway);
        }
    }

    let mut builder = JwtVerifier::builder(config);

    if let Ok(url) = env::var("JWT_JWKS_URL") {
        info!(jwks_url = %url, "Configuring JWKS fetcher");
        builder = builder.with_jwks_url(url);
    }

    if let Ok(pem) = env::var("JWT_DEV_PUBLIC_KEY_PEM") {
        warn!("Using JWT_DEV_PUBLIC_KEY_PEM for verification; do not enable in production");
        builder = builder
            .with_rsa_pem("local-dev", pem.as_bytes())
            .map_err(anyhow::Error::from)?;
    }

    let verifier = builder.build().await.map_err(anyhow::Error::from)?;
    info!("JWT verifier initialised");
    Ok(Arc::new(verifier))
}
fn spawn_jwks_refresh(verifier: Arc<JwtVerifier>) {
    let Some(fetcher) = verifier.jwks_fetcher() else {
        return;
    };

    let refresh_secs = env::var("JWKS_REFRESH_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(300);
    let refresh_secs = refresh_secs.max(60);
    let interval_duration = Duration::from_secs(refresh_secs);
    let url = fetcher.url().to_owned();
    let handle = verifier.clone();

    tokio::spawn(async move {
        let mut ticker = interval(interval_duration);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match handle.refresh_jwks().await {
                Ok(count) => {
                    debug!(count, jwks_url = %url, "Refreshed JWKS keys");
                }
                Err(err) => {
                    warn!(error = %err, jwks_url = %url, "Failed to refresh JWKS keys");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{Path, State},
        http::{HeaderMap, HeaderValue},
    };
    // chrono::Utc no longer needed in this test module after refactor
    use common_auth::{JwtConfig, JwtVerifier};
    use common_security::SecurityContext;
    use common_crypto::{generate_dek, MasterKey};
    // serde_json::json no longer needed in this test module after refactor
    use sqlx::{migrate::MigrateError, PgPool, Row};
    use std::{io, sync::Arc};
    use uuid::Uuid;

    fn require_database_url() -> Option<String> {
        std::env::var("CUSTOMER_TEST_DATABASE_URL")
            .ok()
            .or_else(|| std::env::var("DATABASE_URL").ok())
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres schema migrations)")]
    async fn update_customer_allows_editing_contact_fields(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let database_url = match require_database_url() {
            Some(url) => url,
            None => {
                eprintln!("Skipping customer update test because DATABASE_URL is not set.");
                return Ok(());
            }
        };

        let pool = PgPool::connect(&database_url).await?;
        if let Err(err) = sqlx::migrate!("./migrations").run(&pool).await {
            if !matches!(err, MigrateError::VersionMissing(_)) {
                return Err(err.into());
            }
        }

        let master_key = MasterKey::from_bytes([3u8; 32])?;
        let jwt_verifier = Arc::new(JwtVerifier::new(JwtConfig::new(
            "test-issuer",
            "test-audience",
        )));
        let state = AppState {
            db: pool.clone(),
            jwt_verifier,
            master_key: Arc::new(master_key.clone()),
        };

        let tenant_id = Uuid::new_v4();
        let actor_id = Uuid::new_v4();
        let dek = generate_dek();
        let encrypted_key = master_key.encrypt_tenant_dek(&dek)?;

        sqlx::query(
            "INSERT INTO tenant_data_keys (id, tenant_id, key_version, encrypted_key, active)
             VALUES ($1, $2, $3, $4, TRUE)",
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(1i32)
        .bind(&encrypted_key)
        .execute(&pool)
        .await?;

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-tenant-id",
            HeaderValue::from_str(&tenant_id.to_string())?,
        );

        // Simulate security context (bypassing extractor since we invoke handler directly)
        headers.insert("X-Tenant-ID", HeaderValue::from_str(&tenant_id.to_string())?);
        headers.insert("X-Roles", HeaderValue::from_static("admin"));
        headers.insert("X-User-ID", HeaderValue::from_str(&actor_id.to_string())?);

        // Minimal dummy actor (structure must match expected fields used by downstream code).
        let actor = common_audit::AuditActor { id: Some(actor_id), name: None, email: None };
        let sec = SecurityContext {
            tenant_id,
            actor,
            roles: vec![Role::Admin],
            trace_id: None,
        };

        let created = create_customer(
            State(state.clone()),
            SecurityCtxExtractor(sec.clone()),
            Json(NewCustomer {
                name: "Alice Example".to_string(),
                email: Some("alice@example.com".to_string()),
                phone: Some("+15550001".to_string()),
            }),
        )
        .await
    .map_err(|e| io::Error::other(format!("create_customer failed: {:?}", e)))?
        .0;

        let customer_id = created.id;

        let updated = update_customer(
            State(state.clone()),
            SecurityCtxExtractor(sec.clone()),
            Path(customer_id),
            Json(UpdateCustomerRequest {
                name: Some("Alice Cooper".to_string()),
                email: Some(Some("alice.cooper@example.com".to_string())),
                phone: Some(None),
            }),
        )
        .await
    .map_err(|e| io::Error::other(format!("update_customer failed: {:?}", e)))?
        .0;

        assert_eq!(updated.name, "Alice Cooper");
        assert_eq!(updated.email.as_deref(), Some("alice.cooper@example.com"));
        assert!(updated.phone.is_none());

        let row = sqlx::query(
            "SELECT email_encrypted, phone_encrypted, email_hash, phone_hash, pii_key_version FROM customers WHERE id = $1"
        )
        .bind(customer_id)
        .fetch_one(&pool)
        .await?;

        let email_encrypted: Option<Vec<u8>> = row.get("email_encrypted");
        let email_hash: Option<Vec<u8>> = row.get("email_hash");
        let phone_encrypted: Option<Vec<u8>> = row.get("phone_encrypted");
        let phone_hash: Option<Vec<u8>> = row.get("phone_hash");
        let pii_key_version: Option<i32> = row.get("pii_key_version");

        assert!(email_encrypted.is_some());
        assert!(email_hash.is_some());
        assert!(phone_encrypted.is_none());
        assert!(phone_hash.is_none());
        assert_eq!(pii_key_version, Some(1));

        sqlx::query("DELETE FROM customers WHERE id = $1")
            .bind(customer_id)
            .execute(&pool)
            .await?;
        sqlx::query("DELETE FROM tenant_data_keys WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await?;

        Ok(())
    }
}
