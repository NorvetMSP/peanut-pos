use axum::{routing::get, routing::post, Router};
use std::net::SocketAddr;
use tokio::net::TcpListener;
mod payment_handlers;
use payment_handlers::process_card_payment;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging...
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/payments", post(process_card_payment));
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8086".to_string())
        .parse()?;
    let addr = SocketAddr::new(host.parse()?, port);
    println!("starting payment-service on {}", addr);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
