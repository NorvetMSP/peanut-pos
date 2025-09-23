use anyhow::{anyhow, Context, Result};
use clap::Parser;
use common_crypto::{generate_dek, MasterKey};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(about = "Seed or rotate tenant data encryption keys", long_about = None)]
struct Options {
    /// Tenant IDs to seed keys for (repeatable)
    #[arg(long = "tenant", value_name = "UUID")]
    tenants: Vec<Uuid>,

    /// Rotate even if an active key already exists
    #[arg(long)]
    rotate: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts = Options::parse();
    if opts.tenants.is_empty() {
        return Err(anyhow!("Provide at least one --tenant <UUID>"));
    }

    let database_url =
        std::env::var("DATABASE_URL").context("DATABASE_URL must be set for seeding keys")?;
    let master_raw = std::env::var("CUSTOMER_MASTER_KEY")
        .context("CUSTOMER_MASTER_KEY must be set for seeding keys")?;
    let master = MasterKey::from_base64(&master_raw)
        .map_err(|err| anyhow!("Failed to parse CUSTOMER_MASTER_KEY: {err}"))?;

    let pool = PgPool::connect(&database_url).await?;

    for tenant in &opts.tenants {
        seed_for_tenant(&pool, &master, *tenant, opts.rotate).await?;
    }

    Ok(())
}

async fn seed_for_tenant(
    pool: &PgPool,
    master: &MasterKey,
    tenant_id: Uuid,
    rotate: bool,
) -> Result<()> {
    let mut tx = pool.begin().await?;

    let active = sqlx::query(
        "SELECT key_version FROM tenant_data_keys WHERE tenant_id = $1 AND active = TRUE LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .map(|row| row.get::<i32, _>("key_version"));

    if let Some(current) = active {
        if !rotate {
            println!("tenant {tenant_id}: active key version {current} already present, skipping");
            tx.rollback().await?;
            return Ok(());
        }
    }

    if rotate {
        sqlx::query(
            "UPDATE tenant_data_keys SET active = FALSE, rotated_at = NOW() WHERE tenant_id = $1 AND active = TRUE"
        )
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;
    }

    let current_version: i32 = sqlx::query(
        "SELECT COALESCE(MAX(key_version), 0) AS value FROM tenant_data_keys WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?
    .get("value");

    let new_version = current_version + 1;
    let dek = generate_dek();
    let encrypted = master
        .encrypt_tenant_dek(&dek)
        .map_err(|err| anyhow!("Failed to encrypt tenant DEK: {err}"))?;

    sqlx::query(
        "INSERT INTO tenant_data_keys (id, tenant_id, key_version, encrypted_key, created_at, active)
         VALUES ($1, $2, $3, $4, NOW(), TRUE)"
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(new_version)
    .bind(&encrypted)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    println!(
        "tenant {tenant_id}: inserted key version {new_version}{}",
        if rotate {
            " (rotated previous keys)"
        } else {
            ""
        }
    );

    Ok(())
}
