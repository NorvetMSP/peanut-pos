#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
async fn with_embedded_postgres() -> Result<()> {
    let Some(db) = TestDatabase::setup().await? else {
        return Ok(());
    };

    let pool = db.pool_clone();
    sqlx::query("SELECT 1").execute(&pool).await?;

    db.teardown().await?;
    Ok(())
}
mod support;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use auth_service::config::AuthConfig;
use auth_service::metrics::AuthMetrics;
use auth_service::notifications::KafkaProducer;
use auth_service::tokens::{TokenConfig, TokenSigner};
use auth_service::user_handlers::{login_user, logout_user, refresh_session, LoginRequest};
use auth_service::AppState;
use axum::body::to_bytes;
use axum::response::IntoResponse;
use axum::{
    extract::State,
    http::{
        header::{COOKIE, SET_COOKIE},
        HeaderMap, HeaderValue, StatusCode,
    },
    Json,
};
use chrono::{Duration as ChronoDuration, Utc};
use common_auth::{JwtConfig, JwtVerifier};
use rand_core::OsRng;
use rdkafka::producer::FutureProducer;
use rdkafka::ClientConfig;
use reqwest::Client;
use rsa::pkcs1::EncodeRsaPublicKey;
use rsa::pkcs8::EncodePrivateKey;
use rsa::RsaPrivateKey;
use serde_json::{from_slice, Value};
use sqlx::PgPool;
use support::{default_auth_config, seed_test_user, TestDatabase};
use uuid::Uuid;

#[derive(Default, Clone)]
struct TestOptions {
    require_mfa: bool,
    required_roles: Vec<String>,
    user_role: String,
    mfa_secret: Option<String>,
    failed_attempts: i16,
    lock_duration_minutes: Option<i64>,
}

impl TestOptions {
    fn apply_config(&self, mut config: AuthConfig) -> AuthConfig {
        if self.require_mfa {
            config.require_mfa = true;
        }
        if !self.required_roles.is_empty() {
            config.required_roles = self
                .required_roles
                .iter()
                .map(|role| role.to_ascii_lowercase())
                .collect();
        }
        config
    }

    fn user_role(&self) -> &str {
        if self.user_role.is_empty() {
            "admin"
        } else {
            &self.user_role
        }
    }
}

struct TestContext {
    app_state: AppState,
    pool: PgPool,
    db: TestDatabase,
    tenant_id: Uuid,
    user_id: Uuid,
    email: String,
    password: String,
}

impl TestContext {
    async fn bootstrap() -> Result<Option<Self>> {
        Self::bootstrap_with_options(TestOptions::default()).await
    }

    async fn bootstrap_with_options(options: TestOptions) -> Result<Option<Self>> {
        let Some(db) = TestDatabase::setup().await? else {
            return Ok(None);
        };

        let pool = db.pool_clone();

        let seeded = seed_test_user(&pool, options.user_role()).await?;
        let tenant_id = seeded.tenant_id;
        let user_id = seeded.user_id;
        let email = seeded.email.clone();
        let password = seeded.password.clone();

        if let Some(secret) = &options.mfa_secret {
            sqlx::query("UPDATE users SET mfa_secret = $1, mfa_enrolled_at = NOW() WHERE id = $2")
                .bind(secret)
                .bind(user_id)
                .execute(&pool)
                .await?;
        }

        if options.failed_attempts > 0 || options.lock_duration_minutes.is_some() {
            let locked_until = options
                .lock_duration_minutes
                .map(|mins| Utc::now() + ChronoDuration::minutes(mins));
            sqlx::query("UPDATE users SET failed_attempts = $1, locked_until = $2 WHERE id = $3")
                .bind(options.failed_attempts)
                .bind(locked_until)
                .bind(user_id)
                .execute(&pool)
                .await?;
        }

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

        let kafka_client: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", "localhost:9092")
            .create()
            .context("Failed to create Kafka producer")?;
        let kafka_producer: Arc<dyn KafkaProducer> = Arc::new(kafka_client);

        let http_client = Client::builder().build()?;

        let config = options.apply_config(default_auth_config());

        let state = AppState {
            db: pool.clone(),
            jwt_verifier: Arc::new(verifier),
            token_signer: Arc::new(token_signer),
            config: Arc::new(config),
            kafka_producer,
            http_client,
            metrics: Arc::new(AuthMetrics::new()?),
        };

        Ok(Some(Self {
            app_state: state,
            pool,
            db,
            tenant_id,
            user_id,
            email,
            password,
        }))
    }

    async fn login(&self) -> Result<LoginResult> {
        self.login_with_password(&self.password).await
    }

    async fn login_with_password(&self, password: &str) -> Result<LoginResult> {
        let request = LoginRequest {
            email: self.email.clone(),
            password: password.to_string(),
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

        self.db.teardown().await?;

        Ok(())
    }
}

#[derive(Debug)]
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

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
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
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
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
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
async fn logout_revokes_refresh_token_and_clears_cookie() -> Result<()> {
    let Some(ctx) = TestContext::bootstrap().await? else {
        return Ok(());
    };
    let login = ctx.login().await?;

    let mut headers = HeaderMap::new();
    headers.insert(COOKIE, HeaderValue::from_str(&login.cookie_pair())?);

    let response = logout_user(State(ctx.app_state.clone()), headers.clone()).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let set_cookie = response
        .headers()
        .get(SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow!("missing logout cookie"))?;
    assert!(set_cookie.contains("Max-Age=0"));

    let refresh_attempt = refresh_session(State(ctx.app_state.clone()), headers)
        .await
        .expect_err("expected session expiration after logout");
    let refresh_response = refresh_attempt.into_response();
    assert_eq!(refresh_response.status(), StatusCode::UNAUTHORIZED);

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

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
async fn login_locks_account_after_repeated_failures() -> Result<()> {
    let Some(ctx) = TestContext::bootstrap().await? else {
        return Ok(());
    };

    for attempt in 1..=5 {
        let result = login_user(
            State(ctx.app_state.clone()),
            HeaderMap::new(),
            Json(LoginRequest {
                email: ctx.email.clone(),
                password: "wrong-password".to_string(),
                tenant_id: Some(ctx.tenant_id),
                mfa_code: None,
                device_fingerprint: None,
            }),
        )
        .await;

        let err = result.expect_err("expected authentication failure");
        let response = err.into_response();
        if attempt < 5 {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        } else {
            assert_eq!(response.status(), StatusCode::LOCKED);
        }
    }

    let (failed_attempts, locked_until): (i16, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as("SELECT failed_attempts, locked_until FROM users WHERE id = $1")
            .bind(ctx.user_id)
            .fetch_one(&ctx.pool)
            .await?;
    assert_eq!(failed_attempts, 5);
    assert!(locked_until.is_some());

    ctx.teardown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
async fn login_requires_mfa_when_enforced() -> Result<()> {
    let Some(ctx) = TestContext::bootstrap_with_options(TestOptions {
        require_mfa: true,
        ..Default::default()
    })
    .await?
    else {
        return Ok(());
    };

    let result = login_user(
        State(ctx.app_state.clone()),
        HeaderMap::new(),
        Json(LoginRequest {
            email: ctx.email.clone(),
            password: ctx.password.clone(),
            tenant_id: Some(ctx.tenant_id),
            mfa_code: None,
            device_fingerprint: None,
        }),
    )
    .await;

    let err = result.expect_err("expected MFA requirement");
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    ctx.teardown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
async fn login_rejects_previously_locked_account() -> Result<()> {
    let Some(ctx) = TestContext::bootstrap_with_options(TestOptions {
        failed_attempts: 5,
        lock_duration_minutes: Some(30),
        ..Default::default()
    })
    .await?
    else {
        return Ok(());
    };

    let result = login_user(
        State(ctx.app_state.clone()),
        HeaderMap::new(),
        Json(LoginRequest {
            email: ctx.email.clone(),
            password: ctx.password.clone(),
            tenant_id: Some(ctx.tenant_id),
            mfa_code: None,
            device_fingerprint: None,
        }),
    )
    .await;

    let err = result.expect_err("expected locked account error");
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::LOCKED);

    ctx.teardown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
async fn refresh_session_rejects_reused_cookie() -> Result<()> {
    let Some(ctx) = TestContext::bootstrap().await? else {
        return Ok(());
    };
    let login = ctx.login().await?;

    let mut headers = HeaderMap::new();
    headers.insert(COOKIE, HeaderValue::from_str(&login.cookie_pair())?);

    let response = refresh_session(State(ctx.app_state.clone()), headers.clone())
        .await
        .map_err(|err| anyhow!("{err:?}"))?;
    assert_eq!(response.status(), StatusCode::OK);

    let reused = refresh_session(State(ctx.app_state.clone()), headers)
        .await
        .expect_err("expected session expiry on reused cookie");
    let reused_response = reused.into_response();
    assert_eq!(reused_response.status(), StatusCode::UNAUTHORIZED);

    ctx.teardown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
async fn logout_clears_cookie_and_prevents_refresh() -> Result<()> {
    let Some(ctx) = TestContext::bootstrap().await? else {
        return Ok(());
    };
    let login = ctx.login().await?;

    let mut headers = HeaderMap::new();
    headers.insert(COOKIE, HeaderValue::from_str(&login.cookie_pair())?);

    let response = logout_user(State(ctx.app_state.clone()), headers.clone()).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let set_cookie = response
        .headers()
        .get(SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow!("missing logout cookie"))?;
    assert!(set_cookie.contains("Max-Age=0"));

    let refresh_attempt = refresh_session(State(ctx.app_state.clone()), headers)
        .await
        .expect_err("expected session expiration after logout");
    let refresh_response = refresh_attempt.into_response();
    assert_eq!(refresh_response.status(), StatusCode::UNAUTHORIZED);

    ctx.teardown().await?;
    Ok(())
}
