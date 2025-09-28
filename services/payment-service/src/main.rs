use anyhow::Context;
use axum::{
    extract::FromRef,
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode,
    },
    routing::{get, post},
    Router,
};
use common_auth::{AuthContext, JwtConfig, JwtVerifier};
use std::{env, net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};
use uuid::Uuid;

mod payment_handlers;
use payment_handlers::{process_card_payment, void_card_payment};

const PAYMENT_ROLES: &[&str] = &["super_admin", "admin", "cashier"];

#[derive(Clone)]
struct AppState {
    jwt_verifier: Arc<JwtVerifier>,
}

impl FromRef<AppState> for Arc<JwtVerifier> {
    fn from_ref(state: &AppState) -> Self {
        state.jwt_verifier.clone()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    let state = AppState { jwt_verifier };

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
        .route("/healthz", get(|| async { "ok" }))
        .route("/payments", post(process_card_payment))
        .route("/payments/void", post(void_card_payment))
        .with_state(state)
        .layer(cors);

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| "8086".to_string())
        .parse()?;
    let addr = SocketAddr::new(host.parse()?, port);
    println!("starting payment-service on {addr}");
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

pub(crate) fn ensure_role(
    auth: &AuthContext,
    allowed: &[&str],
) -> Result<(), (StatusCode, String)> {
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

pub(crate) fn tenant_id_from_request(
    headers: &HeaderMap,
    auth: &AuthContext,
) -> Result<Uuid, (StatusCode, String)> {
    let header_value = headers
        .get("X-Tenant-ID")
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing X-Tenant-ID header".to_string(),
        ))?
        .to_str()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid X-Tenant-ID header".to_string(),
            )
        })?
        .trim();
    let tenant_id = Uuid::parse_str(header_value).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid X-Tenant-ID header".to_string(),
        )
    })?;
    if tenant_id != auth.claims.tenant_id {
        return Err((
            StatusCode::FORBIDDEN,
            "Authenticated tenant does not match X-Tenant-ID header".to_string(),
        ));
    }
    Ok(tenant_id)
}
