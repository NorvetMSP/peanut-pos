use std::{collections::HashSet, env, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{anyhow, Context, Result};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use auth_service::config::{AuthConfig, CookieSameSite};
use auth_service::metrics::AuthMetrics;
use auth_service::tokens::{TokenConfig, TokenSigner};
use auth_service::user_handlers::{login_user, logout_user, refresh_session, LoginRequest};
use auth_service::AppState;
use axum::body::to_bytes;
use axum::{
    extract::State,
    http::{
        header::{COOKIE, SET_COOKIE},
        HeaderMap, HeaderValue, StatusCode,
    },
    Json,
};
use common_auth::{JwtConfig, JwtVerifier};
use dirs::cache_dir;
use pg_embed::pg_enums::PgAuthMethod;
use pg_embed::pg_fetch::{PgFetchSettings, PG_V13};
use pg_embed::postgres::{PgEmbed, PgSettings};
use portpicker::pick_unused_port;
use rand_core::OsRng;
use rdkafka::producer::FutureProducer;
use rdkafka::ClientConfig;
use reqwest::Client;
use rsa::pkcs1::EncodeRsaPublicKey;
use rsa::pkcs8::EncodePrivateKey;
use rsa::RsaPrivateKey;
use serde_json::{from_slice, Value};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tempfile::{tempdir, TempDir};
use uuid::Uuid;

struct EmbeddedPg {
    pg: PgEmbed,
    _temp_dir: TempDir,
}

impl EmbeddedPg {
    async fn shutdown(self) {
        let mut pg = self.pg;
        let _ = pg.stop_db().await;
    }
}

struct TestContext {
    app_state: AppState,
    pool: PgPool,
    embedded: Option<EmbeddedPg>,
    tenant_id: Uuid,
    user_id: Uuid,
    password: String,
}

impl TestContext {
    async fn bootstrap() -> Result<Option<Self>> {
        if env::var("AUTH_TEST_DATABASE_URL").is_err()
            && !matches!(env::var("AUTH_TEST_USE_EMBED"), Ok(flag) if matches!(flag.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        {
            eprintln!("Skipping auth-service integration tests: set AUTH_TEST_DATABASE_URL or AUTH_TEST_USE_EMBED=1 to run them.");
            return Ok(None);
        }

        let mut embedded = None;
        let database_url = if let Ok(url) = env::var("AUTH_TEST_DATABASE_URL") {
            url
        } else {
            if matches!(
                env::var("AUTH_TEST_EMBED_CLEAR_CACHE"),
                Ok(flag) if matches!(flag.as_str(), "1" | "true" | "TRUE" | "yes" | "YES")
            ) {
                if let Some(cache_dir) = cache_dir() {
                    let _ = std::fs::remove_dir_all(cache_dir.join("pg-embed"));
                }
            }

            let temp = tempdir()?;
            let port = pick_unused_port().expect("free port");

            let mut fetch_settings = PgFetchSettings::default();
            fetch_settings.version = PG_V13;
            let mut pg = PgEmbed::new(
                PgSettings {
                    database_dir: temp.path().to_path_buf(),
                    port,
                    user: "postgres".to_string(),
                    password: "postgres".to_string(),
                    auth_method: PgAuthMethod::Plain,
                    persistent: false,
                    timeout: Some(Duration::from_secs(30)),
                    migration_dir: None,
                },
                fetch_settings,
            )
            .await?;

            pg.setup().await?;
            pg.start_db().await?;

            let uri = format!("{}/postgres", pg.db_uri);
            embedded = Some(EmbeddedPg {
                pg,
                _temp_dir: temp,
            });
            uri
        };

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;

        let apply_migrations = embedded.is_some()
            || matches!(env::var("AUTH_TEST_APPLY_MIGRATIONS"), Ok(flag) if matches!(flag.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"));
        if apply_migrations {
            run_migrations(&pool).await?;
        }

        let tenant_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let password = "CorrectHorseBatteryStaple!".to_string();
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)?
            .to_string();

        sqlx::query("INSERT INTO tenants (id, name) VALUES ($1, $2)")
            .bind(tenant_id)
            .bind("Test Tenant")
            .execute(&pool)
            .await?;

        sqlx::query(
            "INSERT INTO users (id, tenant_id, name, email, role, password_hash) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(user_id)
        .bind(tenant_id)
        .bind("Test User")
        .bind("user@example.com")
        .bind("admin")
        .bind(&password_hash)
        .execute(&pool)
        .await?;

        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048)?;
        let private_pem = private_key
            .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)?
            .to_string();
        let public_pem = private_key
            .to_public_key()
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)?
            .to_string();

        let jwt_config = JwtConfig::new("test-issuer", "test-audience");
        let verifier = JwtVerifier::builder(jwt_config)
            .with_rsa_pem("local-test", public_pem.as_bytes())?
            .build()
            .await?;

        let token_config = TokenConfig {
            issuer: "test-issuer".to_string(),
            audience: "test-audience".to_string(),
            access_ttl_seconds: 900,
            refresh_ttl_seconds: 7200,
        };
        let token_signer = TokenSigner::new(pool.clone(), token_config, Some(&private_pem)).await?;

        let kafka_producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", "localhost:9092")
            .create()
            .context("Failed to create Kafka producer")?;

        let http_client = Client::builder().build()?;

        let state = AppState {
            db: pool.clone(),
            jwt_verifier: Arc::new(verifier),
            token_signer: Arc::new(token_signer),
            config: Arc::new(test_auth_config()),
            kafka_producer,
            http_client,
            metrics: Arc::new(AuthMetrics::new()?),
        };

        Ok(Some(Self {
            app_state: state,
            pool,
            embedded,
            tenant_id,
            user_id,
            password,
        }))
    }

    async fn login(&self) -> Result<LoginResult> {
        let request = LoginRequest {
            email: "user@example.com".to_string(),
            password: self.password.clone(),
            tenant_id: Some(self.tenant_id),
            mfa_code: None,
            device_fingerprint: Some("device-123".to_string()),
        };

        let response = login_user(
            State(self.app_state.clone()),
            HeaderMap::new(),
            Json(request),
        )
        .await
        .map_err(|err| anyhow!("{err:?}"))?;

        let (parts, body) = response.into_parts();
        let set_cookie = parts
            .headers
            .get(SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| anyhow!("missing refresh cookie"))?;
        let bytes = to_bytes(body, usize::MAX).await?;
        let payload: Value = from_slice(&bytes)?;

        Ok(LoginResult {
            cookie: set_cookie.to_string(),
            payload,
        })
    }

    async fn teardown(self) -> Result<()> {
        sqlx::query("DELETE FROM auth_refresh_tokens WHERE user_id = $1")
            .bind(self.user_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(self.user_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM tenants WHERE id = $1")
            .bind(self.tenant_id)
            .execute(&self.pool)
            .await?;

        if let Some(embedded) = self.embedded {
            embedded.shutdown().await;
        }

        Ok(())
    }
}

struct LoginResult {
    cookie: String,
    payload: Value,
}

impl LoginResult {
    fn cookie_pair(&self) -> String {
        self.cookie
            .split(';')
            .next()
            .map(|value| value.trim().to_string())
            .unwrap_or_else(|| self.cookie.clone())
    }

    fn user_id(&self) -> Option<&str> {
        self.payload["user"]["id"].as_str()
    }

    fn tenant_id(&self) -> Option<&str> {
        self.payload["user"]["tenant_id"].as_str()
    }
}

async fn run_migrations(pool: &PgPool) -> Result<()> {
    let migrations_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut entries = std::fs::read_dir(&migrations_dir)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();

    for path in entries {
        let sql = std::fs::read_to_string(&path)?;
        for statement in sql.split(';') {
            let trimmed = statement.trim();
            if trimmed.is_empty() {
                continue;
            }
            sqlx::query(trimmed).execute(pool).await?;
        }
    }

    Ok(())
}

fn test_auth_config() -> AuthConfig {
    AuthConfig {
        require_mfa: false,
        required_roles: HashSet::new(),
        bypass_tenants: HashSet::new(),
        mfa_issuer: "NovaPOS".to_string(),
        mfa_activity_topic: "security.mfa.activity".to_string(),
        suspicious_webhook_url: None,
        suspicious_webhook_bearer: None,
        refresh_cookie_name: "novapos_refresh".to_string(),
        refresh_cookie_domain: None,
        refresh_cookie_secure: false,
        refresh_cookie_same_site: CookieSameSite::Lax,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires embedded Postgres binary download"]
async fn login_flow_sets_refresh_cookie_and_persists_session() -> Result<()> {
    let Some(ctx) = TestContext::bootstrap().await? else {
        return Ok(());
    };
    let login = ctx.login().await?;

    assert!(login.cookie.contains("novapos_refresh="));
    assert!(login.cookie.contains("HttpOnly"));
    assert!(login.user_id().is_some());
    assert_eq!(login.user_id().unwrap(), ctx.user_id.to_string());
    assert_eq!(login.tenant_id().unwrap(), ctx.tenant_id.to_string());

    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_refresh_tokens WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(ctx.user_id)
    .fetch_one(&ctx.pool)
    .await?;
    assert_eq!(active, 1);

    ctx.teardown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires embedded Postgres binary download"]
async fn refresh_session_rotates_cookie_and_refresh_token() -> Result<()> {
    let Some(ctx) = TestContext::bootstrap().await? else {
        return Ok(());
    };
    let login = ctx.login().await?;

    let mut headers = HeaderMap::new();
    headers.insert(COOKIE, HeaderValue::from_str(&login.cookie_pair())?);

    let response = refresh_session(State(ctx.app_state.clone()), headers)
        .await
        .map_err(|err| anyhow!("{err:?}"))?;
    assert_eq!(response.status(), StatusCode::OK);

    let (parts, body) = response.into_parts();
    let new_cookie = parts
        .headers
        .get(SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow!("missing refreshed cookie"))?;
    assert!(new_cookie.contains("novapos_refresh="));
    assert_ne!(
        login.cookie_pair(),
        new_cookie.split(';').next().unwrap().trim()
    );

    let bytes = to_bytes(body, usize::MAX).await?;
    let payload: Value = from_slice(&bytes)?;
    let expected_user_id = ctx.user_id.to_string();
    assert_eq!(
        payload["user"]["id"].as_str(),
        Some(expected_user_id.as_str())
    );

    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_refresh_tokens WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(ctx.user_id)
    .fetch_one(&ctx.pool)
    .await?;
    assert_eq!(active, 1);

    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM auth_refresh_tokens WHERE user_id = $1")
            .bind(ctx.user_id)
            .fetch_one(&ctx.pool)
            .await?;
    assert_eq!(total, 2);

    ctx.teardown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires embedded Postgres binary download"]
async fn logout_revokes_refresh_token_and_clears_cookie() -> Result<()> {
    let Some(ctx) = TestContext::bootstrap().await? else {
        return Ok(());
    };
    let login = ctx.login().await?;

    let mut headers = HeaderMap::new();
    headers.insert(COOKIE, HeaderValue::from_str(&login.cookie_pair())?);

    let response = logout_user(State(ctx.app_state.clone()), headers).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let set_cookie = response
        .headers()
        .get(SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow!("missing logout cookie"))?;
    assert!(set_cookie.contains("Max-Age=0"));

    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_refresh_tokens WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(ctx.user_id)
    .fetch_one(&ctx.pool)
    .await?;
    assert_eq!(active, 0);

    let revoked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_refresh_tokens WHERE user_id = $1 AND revoked_at IS NOT NULL",
    )
    .bind(ctx.user_id)
    .fetch_one(&ctx.pool)
    .await?;
    assert_eq!(revoked, 1);

    ctx.teardown().await?;
    Ok(())
}
