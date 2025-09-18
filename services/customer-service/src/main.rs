// services/customer-service/src/main.rs
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use std::env;
use std::net::{IpAddr, SocketAddr};
use tokio::net::TcpListener;
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    db: PgPool,
}

type ApiResult<T> = Result<T, (StatusCode, String)>;

#[derive(Deserialize)]
struct NewCustomer {
    name: String,
    email: Option<String>,
    phone: Option<String>,
}

#[derive(Serialize, FromRow)]
struct Customer {
    id: Uuid,
    tenant_id: Uuid,
    name: String,
    email: Option<String>,
    phone: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize, Default)]
struct SearchParams {
    q: Option<String>,
}

async fn create_customer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(new_cust): Json<NewCustomer>,
) -> ApiResult<Json<Customer>> {
    let tenant_id = extract_tenant_id(&headers)?;
    let customer_id = Uuid::new_v4();
    let NewCustomer { name, email, phone } = new_cust;

    let customer = sqlx::query_as::<_, Customer>(
        "INSERT INTO customers (id, tenant_id, name, email, phone)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, tenant_id, name, email, phone, created_at",
    )
    .bind(customer_id)
    .bind(tenant_id)
    .bind(name)
    .bind(email)
    .bind(phone)
    .fetch_one(&state.db)
    .await
    .map_err(internal_err)?;

    Ok(Json(customer))
}

async fn search_customers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Vec<Customer>>> {
    let tenant_id = extract_tenant_id(&headers)?;
    let pattern = params
        .q
        .as_ref()
        .map(|q| format!("%{}%", q))
        .unwrap_or_else(|| "%".to_string());

    let customers = sqlx::query_as::<_, Customer>(
        "SELECT id, tenant_id, name, email, phone, created_at
         FROM customers
         WHERE tenant_id = $1
           AND (name ILIKE $2 OR email ILIKE $2 OR phone ILIKE $2)
         ORDER BY created_at DESC
         LIMIT 20",
    )
    .bind(tenant_id)
    .bind(pattern)
    .fetch_all(&state.db)
    .await
    .map_err(internal_err)?;

    Ok(Json(customers))
}

async fn get_customer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(customer_id): Path<Uuid>,
) -> ApiResult<Json<Customer>> {
    let tenant_id = extract_tenant_id(&headers)?;

    let customer = sqlx::query_as::<_, Customer>(
        "SELECT id, tenant_id, name, email, phone, created_at
         FROM customers
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant_id)
    .bind(customer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_err)?
    .ok_or((StatusCode::NOT_FOUND, "Customer not found".into()))?;

    Ok(Json(customer))
}

fn extract_tenant_id(headers: &HeaderMap) -> ApiResult<Uuid> {
    headers
        .get("X-Tenant-ID")
        .and_then(|h| h.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing or invalid X-Tenant-ID".into(),
        ))
}

fn internal_err(err: sqlx::Error) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("DB error: {}", err),
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let database_url = env::var("DATABASE_URL")?;
    let db_pool = PgPool::connect(&database_url).await?;

    let state = AppState {
        db: db_pool.clone(),
    };

    let app = Router::new()
        .route("/customers", post(create_customer).get(search_customers))
        .route("/customers/:id", get(get_customer))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state);

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
