use anyhow::{anyhow, Context, Result};
use clap::Parser;
use common_crypto::{deterministic_hash, encrypt_field, MasterKey};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(about = "Backfill customer PII encryption columns", long_about = None)]
struct Options {
    /// Limit processing to a single tenant
    #[arg(long = "tenant", value_name = "UUID")]
    tenant: Option<Uuid>,

    /// Number of rows to process per batch
    #[arg(long = "batch-size", default_value_t = 100)]
    batch_size: i64,

    /// Print stats without writing any changes
    #[arg(long = "dry-run")]
    dry_run: bool,
}

#[derive(sqlx::FromRow)]
struct CustomerRow {
    id: Uuid,
    tenant_id: Uuid,
    email: Option<String>,
    phone: Option<String>,
}

#[derive(Clone)]
struct TenantDek {
    version: i32,
    key: [u8; 32],
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts = Options::parse();
    if opts.batch_size <= 0 {
        return Err(anyhow!("--batch-size must be positive"));
    }

    let database_url =
        std::env::var("DATABASE_URL").context("DATABASE_URL must be set for backfill script")?;
    let master_raw = std::env::var("CUSTOMER_MASTER_KEY")
        .context("CUSTOMER_MASTER_KEY must be set for backfill script")?;
    let master = MasterKey::from_base64(&master_raw)
        .map_err(|err| anyhow!("Failed to parse CUSTOMER_MASTER_KEY: {err}"))?;

    let pool = PgPool::connect(&database_url).await?;

    if opts.dry_run {
        let count: i64 = pending_count(&pool, opts.tenant).await?;
        if let Some(tenant) = opts.tenant {
            println!("tenant {tenant}: {count} customers need encryption");
        } else {
            println!("{count} customers across all tenants need encryption");
        }
        return Ok(());
    }

    let mut total = 0usize;
    let mut cache = HashMap::new();

    loop {
        let rows = fetch_batch(&pool, opts.tenant, opts.batch_size).await?;
        if rows.is_empty() {
            break;
        }

        let mut tx = pool.begin().await?;
        for row in &rows {
            let dek = resolve_dek(&mut cache, &pool, &master, row.tenant_id).await?;

            let sanitized_email = sanitize_optional(row.email.clone());
            let sanitized_phone = sanitize_optional(row.phone.clone());

            let email_encrypted = match &sanitized_email {
                Some(value) => Some(
                    encrypt_field(&dek.key, value.as_bytes())
                        .map_err(|err| anyhow!("Failed to encrypt email for {}: {err}", row.id))?,
                ),
                None => None,
            };
            let email_hash =
                match &sanitized_email {
                    Some(value) => {
                        let normalized = normalize_email(value);
                        if normalized.is_empty() {
                            None
                        } else {
                            Some(deterministic_hash(&dek.key, normalized.as_bytes()).map_err(
                                |err| anyhow!("Failed to hash email for {}: {err}", row.id),
                            )?)
                        }
                    }
                    None => None,
                };

            let phone_encrypted = match &sanitized_phone {
                Some(value) => Some(
                    encrypt_field(&dek.key, value.as_bytes())
                        .map_err(|err| anyhow!("Failed to encrypt phone for {}: {err}", row.id))?,
                ),
                None => None,
            };
            let phone_hash =
                match &sanitized_phone {
                    Some(value) => {
                        let normalized = normalize_phone(value);
                        if normalized.is_empty() {
                            None
                        } else {
                            Some(deterministic_hash(&dek.key, normalized.as_bytes()).map_err(
                                |err| anyhow!("Failed to hash phone for {}: {err}", row.id),
                            )?)
                        }
                    }
                    None => None,
                };

            sqlx::query(
                "UPDATE customers
                 SET email_encrypted = $1,
                     phone_encrypted = $2,
                     email_hash = $3,
                     phone_hash = $4,
                     pii_key_version = $5,
                     pii_encrypted_at = NOW()
                 WHERE id = $6",
            )
            .bind(email_encrypted.as_ref())
            .bind(phone_encrypted.as_ref())
            .bind(email_hash.as_ref())
            .bind(phone_hash.as_ref())
            .bind(dek.version)
            .bind(row.id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        total += rows.len();
        println!("Processed {} rows (total {})", rows.len(), total);
    }

    println!("Backfill complete. Updated {total} customers.");
    Ok(())
}

async fn resolve_dek(
    cache: &mut HashMap<Uuid, TenantDek>,
    pool: &PgPool,
    master: &MasterKey,
    tenant_id: Uuid,
) -> Result<TenantDek> {
    if let Some(entry) = cache.get(&tenant_id) {
        return Ok(entry.clone());
    }
    let row = sqlx::query(
        "SELECT key_version, encrypted_key
         FROM tenant_data_keys
         WHERE tenant_id = $1 AND active = TRUE
         ORDER BY key_version DESC
         LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let row =
        row.ok_or_else(|| anyhow!("No active tenant data key found for tenant {tenant_id}"))?;

    let version = row.get::<i32, _>("key_version");
    let encrypted = row.get::<Vec<u8>, _>("encrypted_key");
    let key = master
        .decrypt_tenant_dek(&encrypted)
        .map_err(|err| anyhow!("Failed to decrypt tenant data key for {tenant_id}: {err}"))?;

    let dek = TenantDek { version, key };
    cache.insert(tenant_id, dek.clone());
    Ok(dek)
}

async fn fetch_batch(
    pool: &PgPool,
    tenant: Option<Uuid>,
    batch_size: i64,
) -> Result<Vec<CustomerRow>> {
    let query = if tenant.is_some() {
        sqlx::query_as::<_, CustomerRow>(
            "SELECT id, tenant_id, email, phone
             FROM customers
             WHERE tenant_id = $1
               AND ((email IS NOT NULL AND email_encrypted IS NULL)
                    OR (phone IS NOT NULL AND phone_encrypted IS NULL))
             ORDER BY created_at
             LIMIT $2",
        )
    } else {
        sqlx::query_as::<_, CustomerRow>(
            "SELECT id, tenant_id, email, phone
             FROM customers
             WHERE (email IS NOT NULL AND email_encrypted IS NULL)
                OR (phone IS NOT NULL AND phone_encrypted IS NULL)
             ORDER BY created_at
             LIMIT $1",
        )
    };

    if let Some(tenant_id) = tenant {
        Ok(query
            .bind(tenant_id)
            .bind(batch_size)
            .fetch_all(pool)
            .await?)
    } else {
        Ok(query.bind(batch_size).fetch_all(pool).await?)
    }
}

async fn pending_count(pool: &PgPool, tenant: Option<Uuid>) -> Result<i64> {
    let count = if let Some(tenant_id) = tenant {
        sqlx::query(
            "SELECT COUNT(*) AS value FROM customers
             WHERE tenant_id = $1
               AND ((email IS NOT NULL AND email_encrypted IS NULL)
                    OR (phone IS NOT NULL AND phone_encrypted IS NULL))",
        )
        .bind(tenant_id)
        .fetch_one(pool)
        .await?
        .get::<i64, _>("value")
    } else {
        sqlx::query(
            "SELECT COUNT(*) AS value FROM customers
             WHERE (email IS NOT NULL AND email_encrypted IS NULL)
                OR (phone IS NOT NULL AND phone_encrypted IS NULL)",
        )
        .fetch_one(pool)
        .await?
        .get::<i64, _>("value")
    };
    Ok(count)
}

fn sanitize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn normalize_email(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_phone(value: &str) -> String {
    value.chars().filter(|c| c.is_ascii_digit()).collect()
}
