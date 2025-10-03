use std::collections::{HashSet, VecDeque};
use std::env;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use async_trait::async_trait;
use auth_service::config::{AuthConfig, CookieSameSite};
use auth_service::notifications::KafkaProducer;
use data_encoding::BASE32_NOPAD;
use dirs::cache_dir;
use hmac::{Hmac, Mac};
use pg_embed::pg_enums::PgAuthMethod;
use pg_embed::pg_errors::{PgEmbedError, PgEmbedErrorType};
use pg_embed::pg_fetch::{PgFetchSettings, PG_V13};
use pg_embed::postgres::{PgEmbed, PgSettings};
use portpicker::pick_unused_port;
use rand::rngs::OsRng;
use sha1::Sha1;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tempfile::{tempdir, TempDir};
use uuid::Uuid;

type HmacSha1 = Hmac<Sha1>;
const TOTP_PERIOD_SECONDS: u64 = 30;
const TOTP_DIGITS: u32 = 6;
const DEFAULT_DOCKER_DATABASE_URL: &str = "postgres://novapos:novapos@localhost:5432/novapos";

pub struct TestDatabase {
    pool: PgPool,
    embedded: Option<EmbeddedPg>,
    #[allow(dead_code)]
    database_url: String,
}

impl TestDatabase {
    pub async fn setup() -> Result<Option<Self>> {
        let database_url = determine_database_url()?;
        let mut embedded = None;

        let database_url = if let DatabaseSource::Provided(url) = database_url {
            url
        } else {
            if env_flag_enabled("AUTH_TEST_EMBED_CLEAR_CACHE") {
                clear_pg_embed_cache();
            }

            let port = pick_unused_port()
                .context("failed to find available port for embedded Postgres")?;

            let mut retried_after_cache_clear = false;

            let (pg, temp_dir, uri) = loop {
                let temp = tempdir()?;

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

                match pg.setup().await {
                    Ok(()) => {
                        pg.start_db().await.map_err(anyhow::Error::from)?;
                        let uri = format!("{}/postgres", pg.db_uri);
                        break (pg, temp, uri);
                    }
                    Err(err) => {
                        if should_retry_pg_embed(&err) {
                            if !retried_after_cache_clear {
                                retried_after_cache_clear = true;
                                clear_pg_embed_cache();
                                continue;
                            } else {
                                let message = err.to_string();
                                eprintln!(
                                    "Skipping auth-service integration tests: {message}. Set AUTH_TEST_DATABASE_URL to reuse an existing Postgres instance."
                                );
                                return Ok(None);
                            }
                        }
                        return Err(err.into());
                    }
                }
            };

            embedded = Some(EmbeddedPg {
                pg,
                _temp_dir: temp_dir,
            });
            uri
        };

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;

        // Auto-run migrations when:
        //  * Using embedded Postgres, OR
        //  * Explicit opt-in flag AUTH_TEST_APPLY_MIGRATIONS is set, OR
        //  * We are using the default docker DSN (heuristic: matches DEFAULT_DOCKER_DATABASE_URL)
        // Unless explicit opt-out flag AUTH_TEST_SKIP_AUTO_MIGRATIONS is present.
        let default_docker = database_url.starts_with(DEFAULT_DOCKER_DATABASE_URL);
        if !env_flag_enabled("AUTH_TEST_SKIP_AUTO_MIGRATIONS")
            && (embedded.is_some()
                || env_flag_enabled("AUTH_TEST_APPLY_MIGRATIONS")
                || default_docker)
        {
            if let Err(e) = run_migrations(&pool).await {
                eprintln!("[auth-service test] migration error: {e}");
                return Err(e);
            }
        } else {
            eprintln!(
                "[auth-service test] skipping migrations (set AUTH_TEST_APPLY_MIGRATIONS=1 or unset AUTH_TEST_SKIP_AUTO_MIGRATIONS)"
            );
        }

        Ok(Some(Self {
            pool,
            embedded,
            database_url,
        }))
    }

    pub fn pool_clone(&self) -> PgPool {
        self.pool.clone()
    }

    #[allow(dead_code)]
    pub fn url(&self) -> &str {
        &self.database_url
    }

    pub async fn teardown(self) -> Result<()> {
        if let Some(embedded) = self.embedded {
            embedded.shutdown().await;
        }
        Ok(())
    }
}

enum DatabaseSource {
    Provided(String),
    Embedded,
}

fn determine_database_url() -> Result<DatabaseSource> {
    if let Ok(url) = env::var("AUTH_TEST_DATABASE_URL") {
        return Ok(DatabaseSource::Provided(url));
    }

    if env_flag_enabled("AUTH_TEST_USE_EMBED") {
        return Ok(DatabaseSource::Embedded);
    }

    eprintln!(
        "Using default Docker Postgres connection string: {}",
        DEFAULT_DOCKER_DATABASE_URL
    );
    env::set_var("AUTH_TEST_DATABASE_URL", DEFAULT_DOCKER_DATABASE_URL);
    Ok(DatabaseSource::Provided(
        DEFAULT_DOCKER_DATABASE_URL.to_string(),
    ))
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

fn clear_pg_embed_cache() {
    if let Some(cache_dir) = cache_dir() {
        let _ = std::fs::remove_dir_all(cache_dir.join("pg-embed"));
    }
}

fn should_retry_pg_embed(err: &PgEmbedError) -> bool {
    if err.error_type != PgEmbedErrorType::ReadFileError {
        return false;
    }

    err.to_string().contains("InvalidArchive")
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
            // Attempt execution; ignore idempotent object-exists errors so reruns don't fail.
            match sqlx::query(trimmed).execute(pool).await {
                Ok(_) => {}
                Err(e) => {
                    let upper = trimmed.to_uppercase();
                    let msg = e.to_string();
                    // Detect postgres duplicate object / already exists style errors.
                    let mut duplicate = msg.contains("already exists")
                        || msg.contains("duplicate key value violates unique constraint");
                    // Inspect database error code when available (42710 = duplicate_object).
                    if let sqlx::Error::Database(db_err) = &e {
                        if let Some(code) = db_err.code() {
                            if code == "42710" || code == "42P07" { // duplicate_object, duplicate_table
                                duplicate = true;
                            }
                        }
                    }
                    // Treat CREATE or ALTER TABLE ADD CONSTRAINT as idempotent if duplicate.
                    let is_schema_change = upper.starts_with("CREATE ")
                        || upper.starts_with("ALTER TABLE")
                        || upper.starts_with("CREATE INDEX")
                        || upper.starts_with("CREATE UNIQUE INDEX");
                    if duplicate && is_schema_change {
                        eprintln!("[auth-service test] ignoring duplicate schema element error: {msg}");
                        continue;
                    }
                    return Err(e.into());
                }
            }
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
pub fn current_totp_code(secret: &str) -> Result<String> {
    let secret_bytes = BASE32_NOPAD
        .decode(secret.trim().to_ascii_uppercase().as_bytes())
        .context("invalid TOTP secret encoding")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before UNIX_EPOCH")?
        .as_secs();
    let counter = now / TOTP_PERIOD_SECONDS;

    let code = hotp(&secret_bytes, counter);
    Ok(format!("{:0width$}", code, width = TOTP_DIGITS as usize))
}

#[allow(dead_code)]
fn hotp(secret: &[u8], counter: u64) -> u32 {
    let mut mac = HmacSha1::new_from_slice(secret).expect("HMAC can take key of any size");
    mac.update(&counter.to_be_bytes());
    let result = mac.finalize().into_bytes();

    let offset = (result[result.len() - 1] & 0x0f) as usize;
    let code = ((result[offset] as u32 & 0x7f) << 24)
        | ((result[offset + 1] as u32) << 16)
        | ((result[offset + 2] as u32) << 8)
        | (result[offset + 3] as u32);

    code % 10u32.pow(TOTP_DIGITS)
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
        mfa_dead_letter_topic: Some("security.mfa.activity.dlq".to_string()),
        suspicious_webhook_url: None,
        suspicious_webhook_bearer: None,
        refresh_cookie_name: "novapos_refresh".to_string(),
        refresh_cookie_domain: None,
        refresh_cookie_secure: false,
        refresh_cookie_same_site: CookieSameSite::Lax,
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RecordedKafkaEvent {
    pub topic: String,
    pub key: String,
    pub payload: String,
}

#[derive(Clone, Default)]
#[allow(dead_code)]
pub struct RecordingKafkaProducer {
    events: Arc<Mutex<Vec<RecordedKafkaEvent>>>,
    failures: Arc<Mutex<VecDeque<String>>>,
}

#[allow(dead_code)]
impl RecordingKafkaProducer {
    pub fn drain(&self) -> Vec<RecordedKafkaEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }

    #[allow(dead_code)]
    pub fn fail_next(&self, message: impl Into<String>) {
        self.fail_times(message, 1);
    }

    pub fn fail_times(&self, message: impl Into<String>, count: usize) {
        let message = message.into();
        let mut failures = self.failures.lock().unwrap();
        for _ in 0..count {
            failures.push_back(message.clone());
        }
    }
}

#[async_trait]
impl KafkaProducer for RecordingKafkaProducer {
    async fn send(&self, topic: &str, key: &str, payload: String) -> Result<()> {
        if let Some(message) = self.failures.lock().unwrap().pop_front() {
            return Err(anyhow!(message));
        }
        self.events.lock().unwrap().push(RecordedKafkaEvent {
            topic: topic.to_string(),
            key: key.to_string(),
            payload,
        });
        Ok(())
    }
}

fn env_flag_enabled(key: &str) -> bool {
    matches!(env::var(key), Ok(value) if is_truthy(value.as_str()))
}

fn is_truthy(value: &str) -> bool {
    matches!(value, "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
}
