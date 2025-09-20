use anyhow::Context;
use axum::{
    extract::{FromRef, State},
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method, StatusCode,
    },
    routing::{get, post},
    Json, Router,
};
use common_auth::{JwtConfig, JwtVerifier};
use serde::Serialize;
use sqlx::PgPool;
use std::{env, net::SocketAddr, sync::Arc};
use tokio::{
    net::TcpListener,
    time::{interval, Duration, MissedTickBehavior},
};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};

mod tenant_handlers;
mod tokens;
mod user_handlers;

use tenant_handlers::{
    create_integration_key, create_tenant, list_integration_keys, list_tenants,
    revoke_integration_key,
};
use tokens::{JwkKey, TokenConfig, TokenSigner};
use user_handlers::{create_user, list_roles, list_users, login_user};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt_verifier: Arc<JwtVerifier>,
    pub token_signer: Arc<TokenSigner>,
}

impl FromRef<AppState> for Arc<JwtVerifier> {
    fn from_ref(state: &AppState) -> Self {
        state.jwt_verifier.clone()
    }
}

impl FromRef<AppState> for Arc<TokenSigner> {
    fn from_ref(state: &AppState) -> Self {
        state.token_signer.clone()
    }
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db_pool = PgPool::connect(&database_url).await?;

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    let token_signer = build_token_signer_from_env(&db_pool).await?;

    let state = AppState {
        db: db_pool,
        jwt_verifier,
        token_signer,
    };

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
        .route("/jwks", get(jwks))
        .route("/.well-known/jwks.json", get(jwks))
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

async fn build_token_signer_from_env(db_pool: &PgPool) -> anyhow::Result<Arc<TokenSigner>> {
    let issuer = env::var("JWT_ISSUER").context("JWT_ISSUER must be set")?;
    let audience = env::var("JWT_AUDIENCE").context("JWT_AUDIENCE must be set")?;

    let access_ttl = env::var("TOKEN_ACCESS_TTL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(900);
    let refresh_ttl = env::var("TOKEN_REFRESH_TTL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(2_592_000);

    info!(access_ttl, refresh_ttl, "Configuring token TTLs");

    let fallback_private = env::var("JWT_DEV_PRIVATE_KEY_PEM").ok();

    let config = TokenConfig {
        issuer,
        audience,
        access_ttl_seconds: access_ttl,
        refresh_ttl_seconds: refresh_ttl,
    };

    let signer = TokenSigner::new(db_pool.clone(), config, fallback_private.as_deref()).await?;
    info!("Token signer initialised");
    Ok(Arc::new(signer))
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

async fn jwks(State(state): State<AppState>) -> Result<Json<JwksResponse>, StatusCode> {
    let signer = state.token_signer.clone();
    match signer.jwks().await {
        Ok(keys) => Ok(Json(JwksResponse { keys })),
        Err(err) => {
            warn!(error = %err, "Unable to load JWKS");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[derive(Serialize)]
struct JwksResponse {
    keys: Vec<JwkKey>,
}
