use std::{collections::HashSet, env, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use auth_service::config::{AuthConfig, CookieSameSite};
use dirs::cache_dir;
use pg_embed::pg_enums::PgAuthMethod;
use pg_embed::pg_fetch::{PgFetchSettings, PG_V13};
use pg_embed::postgres::{PgEmbed, PgSettings};
use portpicker::pick_unused_port;
use rand_core::OsRng;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tempfile::{tempdir, TempDir};
use uuid::Uuid;

pub struct TestDatabase {
    pool: PgPool,
    embedded: Option<EmbeddedPg>,
}

impl TestDatabase {
    pub async fn setup() -> Result<Option<Self>> {
        if env::var("AUTH_TEST_DATABASE_URL").is_err() && !env_flag_enabled("AUTH_TEST_USE_EMBED") {
            eprintln!(
                "Skipping auth-service integration tests: set AUTH_TEST_DATABASE_URL or AUTH_TEST_USE_EMBED=1 to run them.",
            );
            return Ok(None);
        }

        let mut embedded = None;
        let database_url = if let Ok(url) = env::var("AUTH_TEST_DATABASE_URL") {
            url
        } else {
            if env_flag_enabled("AUTH_TEST_EMBED_CLEAR_CACHE") {
                if let Some(cache_dir) = cache_dir() {
                    let _ = std::fs::remove_dir_all(cache_dir.join("pg-embed"));
                }
            }

            let temp = tempdir()?;
            let port = pick_unused_port()
                .context("failed to find available port for embedded Postgres")?;

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

        if embedded.is_some() || env_flag_enabled("AUTH_TEST_APPLY_MIGRATIONS") {
            run_migrations(&pool).await?;
        }

        Ok(Some(Self { pool, embedded }))
    }

    pub fn pool_clone(&self) -> PgPool {
        self.pool.clone()
    }

    pub async fn teardown(self) -> Result<()> {
        if let Some(embedded) = self.embedded {
            embedded.shutdown().await;
        }
        Ok(())
    }
}

struct EmbeddedPg {
    pg: PgEmbed,
    _temp_dir: TempDir,
}

impl EmbeddedPg {
    async fn shutdown(mut self) {
        let _ = self.pg.stop_db().await;
    }
}

pub async fn run_migrations(pool: &PgPool) -> Result<()> {
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

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SeededUser {
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub email: String,
    pub password: String,
}

#[allow(dead_code)]
pub async fn seed_test_user(pool: &PgPool, role: &str) -> Result<SeededUser> {
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = "user@example.com".to_string();
    let password = "CorrectHorseBatteryStaple!".to_string();
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string();

    sqlx::query("INSERT INTO tenants (id, name) VALUES ($1, $2)")
        .bind(tenant_id)
        .bind("Test Tenant")
        .execute(pool)
        .await?;

    sqlx::query(
        "INSERT INTO users (id, tenant_id, name, email, role, password_hash) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind("Test User")
    .bind(&email)
    .bind(role)
    .bind(&password_hash)
    .execute(pool)
    .await?;

    Ok(SeededUser {
        tenant_id,
        user_id,
        email,
        password,
    })
}

#[allow(dead_code)]
pub fn default_auth_config() -> AuthConfig {
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

fn env_flag_enabled(key: &str) -> bool {
    matches!(env::var(key), Ok(value) if is_truthy(value.as_str()))
}

fn is_truthy(value: &str) -> bool {
    matches!(value, "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
}
