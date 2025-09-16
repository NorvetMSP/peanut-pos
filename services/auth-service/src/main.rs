use axum::{routing::{get, post}, Router};
use tokio::net::TcpListener;
use std::net::SocketAddr;
use sqlx::PgPool;
use std::env;

mod user_handlers;
use user_handlers::{create_user, list_users};

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

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db_pool = PgPool::connect(&database_url).await?;
    let state = AppState { db: db_pool };

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/users", post(create_user).get(list_users))
        .with_state(state);

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8085);
    let ip: std::net::IpAddr = host.parse()?;
    let addr = SocketAddr::from((ip, port));

    println!("starting auth-service on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
