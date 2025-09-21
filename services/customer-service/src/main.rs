use anyhow::{anyhow, Context};
use axum::{
    extract::{FromRef, Path, Query, State},
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode,
    },
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use common_auth::{AuthContext, JwtConfig, JwtVerifier};
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
use tracing::{debug, error, info, warn};
use uuid::Uuid;

const CUSTOMER_WRITE_ROLES: &[&str] = &["super_admin", "admin", "manager", "cashier"];
const CUSTOMER_VIEW_ROLES: &[&str] = &["super_admin", "admin", "manager", "cashier"];
const GDPR_MANAGE_ROLES: &[&str] = &["super_admin", "admin"];
const GDPR_DELETED_NAME: &str = "[deleted]";

#[derive(Clone)]
struct AppState {
    db: PgPool,
    jwt_verifier: Arc<JwtVerifier>,
    master_key: Arc<MasterKey>,
}

type ApiResult<T> = Result<T, (StatusCode, String)>;

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
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([
            ACCEPT,
            CONTENT_TYPE,
            HeaderName::from_static("authorization"),
            HeaderName::from_static("x-tenant-id"),
        ]);

    let app = Router::new()
        .route("/customers", post(create_customer).get(search_customers))
        .route("/customers/:id", get(get_customer))
        .route("/customers/:id/gdpr/export", post(gdpr_export_customer))
        .route("/customers/:id/gdpr/delete", post(gdpr_delete_customer))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
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

async fn create_customer(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Json(new_cust): Json<NewCustomer>,
) -> ApiResult<Json<Customer>> {
    ensure_role(&auth, CUSTOMER_WRITE_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;
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
    .map_err(internal_err)?;

    let customer = hydrate_customer_row(row, &mut key_cache).await?;
    Ok(Json(customer))
}

async fn search_customers(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Vec<Customer>>> {
    ensure_role(&auth, CUSTOMER_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

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
    .map_err(internal_err)?;

    let customers = hydrate_customer_rows(rows, &mut key_cache).await?;
    Ok(Json(customers))
}

async fn get_customer(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Path(customer_id): Path<Uuid>,
) -> ApiResult<Json<Customer>> {
    ensure_role(&auth, CUSTOMER_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

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
    .map_err(internal_err)?
    .ok_or((StatusCode::NOT_FOUND, "Customer not found".into()))?;

    let customer = hydrate_customer_row(row, &mut key_cache).await?;
    Ok(Json(customer))
}

async fn gdpr_export_customer(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Path(customer_id): Path<Uuid>,
) -> ApiResult<Json<GdprExportResponse>> {
    ensure_role(&auth, GDPR_MANAGE_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

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
    .map_err(internal_err)?
    .ok_or((StatusCode::NOT_FOUND, "Customer not found".into()))?;

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
        Some(auth.claims.subject),
        metadata,
    )
    .await
    .map_err(internal_err)?;

    info!(tenant_id = %tenant_id, customer_id = %customer_id, export_id = %export_id, "GDPR export completed");
    Ok(Json(GdprExportResponse {
        export_id,
        customer,
    }))
}

async fn gdpr_delete_customer(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
    Path(customer_id): Path<Uuid>,
) -> ApiResult<Json<GdprDeleteResponse>> {
    ensure_role(&auth, GDPR_MANAGE_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

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
    .map_err(internal_err)?
    .ok_or((StatusCode::NOT_FOUND, "Customer not found".into()))?;

    let had_email = row.email.is_some() || row.email_encrypted.is_some();
    let had_phone = row.phone.is_some() || row.phone_encrypted.is_some();

    let mut tx = state.db.begin().await.map_err(internal_err)?;
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
    .map_err(internal_err)?;

    if result.rows_affected() == 0 {
        tx.rollback().await.map_err(internal_err)?;
        return Err((StatusCode::NOT_FOUND, "Customer not found".into()));
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
        Some(auth.claims.subject),
        metadata,
    )
    .await
    .map_err(internal_err)?;

    tx.commit().await.map_err(internal_err)?;
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
    let version = key_version.ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Encrypted value missing key version".into(),
    ))?;
    let key = key_cache.by_version(version).await?;
    let plaintext = decrypt_field(&key, &ciphertext).map_err(crypto_err)?;
    String::from_utf8(plaintext).map(Some).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Decrypted value was not valid UTF-8".into(),
        )
    })
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
        .map_err(internal_err)?
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
        .map_err(internal_err)?
    };

    let row = row.ok_or_else(|| {
        let scope = match version {
            Some(v) => format!("version {v}"),
            None => "active".to_string(),
        };
        warn!(tenant_id = %tenant_id, scope = %scope, "Missing tenant data key");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("No {scope} tenant data key found for tenant {tenant_id}"),
        )
    })?;

    let TenantKeyRow {
        key_version,
        encrypted_key,
    } = row;

    let key = master
        .decrypt_tenant_dek(&encrypted_key)
        .map_err(|err| {
            error!(tenant_id = %tenant_id, version = key_version, error = ?err, "Failed to decrypt tenant data key");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to decrypt tenant data key".into(),
            )
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

fn crypto_err(err: CryptoError) -> (StatusCode, String) {
    error!(error = ?err, "Crypto operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Crypto operation failed".into(),
    )
}

fn internal_err(err: sqlx::Error) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("DB error: {}", err),
    )
}

fn ensure_role(auth: &AuthContext, allowed: &[&str]) -> ApiResult<()> {
    let has_role = auth
        .claims
        .roles
        .iter()
        .any(|role| allowed.iter().any(|required| role == required));
    if has_role {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            format!("Insufficient role. Required one of: {}", allowed.join(", ")),
        ))
    }
}

fn tenant_id_from_request(headers: &HeaderMap, auth: &AuthContext) -> ApiResult<Uuid> {
    let header_value = headers
        .get("X-Tenant-ID")
        .ok_or((StatusCode::BAD_REQUEST, "Missing X-Tenant-ID".into()))?
        .to_str()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID".into()))?
        .trim();
    let tenant_id = Uuid::parse_str(header_value)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid X-Tenant-ID".into()))?;
    if tenant_id != auth.claims.tenant_id {
        return Err((
            StatusCode::FORBIDDEN,
            "Authenticated tenant does not match X-Tenant-ID header".into(),
        ));
    }
    Ok(tenant_id)
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
