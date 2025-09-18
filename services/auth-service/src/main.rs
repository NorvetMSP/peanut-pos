use axum::{
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method,
    },
    routing::{get, post},
    Router,
};
use sqlx::PgPool;
use std::env;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::cors::{AllowOrigin, CorsLayer};

mod tenant_handlers;
mod user_handlers;

use tenant_handlers::{
    create_integration_key, create_tenant, list_integration_keys, list_tenants,
    revoke_integration_key,
};
use user_handlers::{create_user, list_roles, list_users, login_user};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db_pool = PgPool::connect(&database_url).await?;
    let state = AppState { db: db_pool };

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list([
            HeaderValue::from_static("http://localhost:3000"),
            HeaderValue::from_static("http://localhost:3001"),
            HeaderValue::from_static("http://localhost:5173"),
        ]))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            ACCEPT,
            CONTENT_TYPE,
            HeaderName::from_static("authorization"),
            HeaderName::from_static("x-tenant-id"),
        ]);

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/login", post(login_user))
        .route("/users", post(create_user).get(list_users))
        .route("/roles", get(list_roles))
        .route("/tenants", post(create_tenant).get(list_tenants))
        .route(
            "/tenants/:tenant_id/integration-keys",
            post(create_integration_key).get(list_integration_keys),
        )
        .route(
            "/integration-keys/:key_id/revoke",
            post(revoke_integration_key),
        )
        .with_state(state)
        .layer(cors);

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8085);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));

    println!("starting auth-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
