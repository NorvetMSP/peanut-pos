mod support;

use anyhow::{anyhow, Result};
use auth_service::tokens::{TokenConfig, TokenSigner, TokenSubject};
use rand::rngs::OsRng;
use rsa::pkcs8::EncodePrivateKey;
use rsa::RsaPrivateKey;
use sqlx::PgPool;
use support::TestDatabase;
use uuid::Uuid;

fn token_config() -> TokenConfig {
    TokenConfig {
        issuer: "test-issuer".to_string(),
        audience: "test-audience".to_string(),
        access_ttl_seconds: 900,
        refresh_ttl_seconds: 7200,
    }
}

fn generate_private_pem() -> Result<String> {
    let mut rng = OsRng;
    let private_key = RsaPrivateKey::new(&mut rng, 2048)?;
    Ok(private_key
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)?
        .to_string())
}

async fn seed_user(pool: &PgPool) -> Result<(Uuid, Uuid)> {
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    sqlx::query("INSERT INTO tenants (id, name) VALUES ($1, $2)")
        .bind(tenant_id)
        .bind("Token Test Tenant")
        .execute(pool)
        .await?;

    sqlx::query(
        "INSERT INTO users (id, tenant_id, name, email, role, password_hash) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind("Token Test User")
    .bind("token-user@example.com")
    .bind("admin")
    .bind("not-used-in-tests")
    .execute(pool)
    .await?;

    Ok((tenant_id, user_id))
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
async fn token_signer_new_requires_signing_key() -> Result<()> {
    let Some(db) = TestDatabase::setup().await? else {
        return Ok(());
    };
    let pool = db.pool_clone();

    let result = TokenSigner::new(pool, token_config(), None).await;
    let err = match result {
        Ok(_) => {
            db.teardown().await?;
            return Err(anyhow!("expected missing signing key error"));
        }
        Err(err) => err,
    };
    assert!(err.to_string().contains("No signing key configured"));

    db.teardown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
async fn jwks_returns_fallback_when_no_active_db_keys() -> Result<()> {
    let Some(db) = TestDatabase::setup().await? else {
        return Ok(());
    };
    let pool = db.pool_clone();

    sqlx::query("DELETE FROM auth_signing_keys")
        .execute(&pool)
        .await?;

    let private_pem = generate_private_pem()?;
    let signer = TokenSigner::new(pool.clone(), token_config(), Some(&private_pem)).await?;

    let keys = signer.jwks().await?;
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].kid, "local-dev");

    db.teardown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
async fn consume_refresh_token_returns_none_for_unknown_token() -> Result<()> {
    let Some(db) = TestDatabase::setup().await? else {
        return Ok(());
    };
    let pool = db.pool_clone();

    let private_pem = generate_private_pem()?;
    let signer = TokenSigner::new(pool.clone(), token_config(), Some(&private_pem)).await?;

    let empty = signer.consume_refresh_token("").await?;
    assert!(empty.is_none());

    let missing = signer
        .consume_refresh_token("f17c7f0c-8a42-4a7a-a1d9-unknown")
        .await?;
    assert!(missing.is_none());

    db.teardown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres: embedded or external)")]
async fn consume_refresh_token_revokes_on_second_use() -> Result<()> {
    let Some(db) = TestDatabase::setup().await? else {
        return Ok(());
    };
    let pool = db.pool_clone();

    let private_pem = generate_private_pem()?;
    let signer = TokenSigner::new(pool.clone(), token_config(), Some(&private_pem)).await?;

    let (tenant_id, user_id) = seed_user(&pool).await?;

    let issued = signer
        .issue_tokens(TokenSubject {
            user_id,
            tenant_id,
            roles: vec!["admin".to_string()],
        })
        .await?;

    let first = signer.consume_refresh_token(&issued.refresh_token).await?;
    assert!(first.is_some());

    let second = signer.consume_refresh_token(&issued.refresh_token).await?;
    assert!(second.is_none());

    sqlx::query("DELETE FROM auth_refresh_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await?;

    db.teardown().await?;
    Ok(())
}
