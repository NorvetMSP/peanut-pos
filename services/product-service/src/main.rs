use axum::{
    routing::{get, post},
    Router,
};
use sqlx::PgPool;
use std::env;
use std::net::SocketAddr;
use tokio::net::TcpListener;
mod product_handlers;
use product_handlers::{create_product, list_products};
/// Shared application state
#[derive(Clone)]
pub struct AppState {
    db: PgPool,
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    // Initialize database connection pool
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db = PgPool::connect(&database_url).await?;
    // Ensure database schema is up to date before serving traffic
    sqlx::migrate!("./migrations").run(&db).await?;
    // Build application state
    let state = AppState { db };
    // Build application routes
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/products", post(create_product).get(list_products))
        .with_state(state);
    // Start server
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8081);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));
    println!("starting product-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
