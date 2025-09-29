use anyhow::Context;
use auth_service::AppState;
use axum::{
    extract::State,
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderName, HeaderValue, Method, StatusCode,
    },
    response::Response,
    routing::{get, post, put},
    Json, Router,
};
use common_auth::{JwtConfig, JwtVerifier};
use rdkafka::producer::FutureProducer;
use reqwest::Client;
use serde::Serialize;
use sqlx::PgPool;
use std::{env, fs, net::SocketAddr, sync::Arc};
use tokio::{
    net::TcpListener,
    time::{interval, Duration, MissedTickBehavior},
};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, info, warn};

use auth_service::config::load_auth_config;
use auth_service::metrics::AuthMetrics;
use auth_service::mfa_handlers::{begin_mfa_enrollment, verify_mfa_enrollment};
use auth_service::notifications::KafkaProducer;
use auth_service::tenant_handlers::{
    create_integration_key, create_tenant, list_integration_keys, list_tenants,
    revoke_integration_key,
};
use auth_service::tokens::{JwkKey, TokenConfig, TokenSigner};
use auth_service::user_handlers::{
    create_user, list_roles, list_users, login_user, logout_user, refresh_session,
    reset_user_password, update_user,
};

async fn health() -> &'static str {
    "ok"
}

async fn metrics_endpoint(State(state): State<AppState>) -> Response {
    match state.metrics.render() {
        Ok(resp) => resp,
        Err(err) => {
            warn!(?err, "Failed to render metrics");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(CONTENT_TYPE, HeaderValue::from_static("text/plain"))
                .body(axum::body::Body::from("metrics unavailable"))
                .expect("failed to build metrics response")
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db_pool = PgPool::connect(&database_url).await?;

    let jwt_verifier = build_jwt_verifier_from_env().await?;
    spawn_jwks_refresh(jwt_verifier.clone());

    let token_signer = build_token_signer_from_env(&db_pool).await?;

    let auth_config = Arc::new(load_auth_config()?);
    let enforced_roles = auth_config.required_roles_sorted().join(",");
    let bypass_tenants = auth_config
        .bypass_tenants_sorted()
        .into_iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    info!(
        require_mfa = auth_config.require_mfa,
        enforced_roles = %enforced_roles,
        bypass_tenants = %bypass_tenants,
        issuer = %auth_config.mfa_issuer,
        "Loaded auth-service configuration"
    );

    let kafka_bootstrap = env::var("KAFKA_BOOTSTRAP")
        .or_else(|_| env::var("KAFKA_BROKERS"))
        .unwrap_or_else(|_| "localhost:9092".to_string());

    let kafka_client: FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", &kafka_bootstrap)
        .create()
        .context("Failed to create Kafka producer")?;
    let kafka_producer: Arc<dyn KafkaProducer> = Arc::new(kafka_client);

    let http_client = Client::builder()
        .build()
        .context("Failed to build HTTP client")?;

    let state = AppState {
        db: db_pool,
        jwt_verifier,
        token_signer,
        config: auth_config.clone(),
        kafka_producer,
        http_client,
        metrics: Arc::new(AuthMetrics::new()?),
    };

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list([
            HeaderValue::from_static("http://localhost:3000"),
            HeaderValue::from_static("http://localhost:3001"),
            HeaderValue::from_static("http://localhost:5173"),
        ]))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::OPTIONS,
        ])
        .allow_headers([
            ACCEPT,
            CONTENT_TYPE,
            HeaderName::from_static("authorization"),
            HeaderName::from_static("x-tenant-id"),
        ])
        .allow_credentials(true);

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/metrics", get(metrics_endpoint))
        .route("/jwks", get(jwks))
        .route("/.well-known/jwks.json", get(jwks))
        .route("/login", post(login_user))
        .route("/session", get(refresh_session))
        .route("/logout", post(logout_user))
        .route("/mfa/enroll", post(begin_mfa_enrollment))
        .route("/mfa/verify", post(verify_mfa_enrollment))
        .route("/users", post(create_user).get(list_users))
        .route("/users/:user_id", put(update_user).patch(update_user))
        .route("/users/:user_id/reset-password", post(reset_user_password))
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

    if let Some(pem) = read_secret_env("JWT_DEV_PUBLIC_KEY_PEM")? {
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

    let fallback_private = read_secret_env("JWT_DEV_PRIVATE_KEY_PEM")?;

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

fn read_secret_env(key: &str) -> anyhow::Result<Option<String>> {
    let file_var = format!("{}_FILE", key);
    if let Ok(path) = env::var(&file_var) {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {} from {}", file_var, path))?;
        return Ok(Some(contents));
    }
    Ok(env::var(key).ok())
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
