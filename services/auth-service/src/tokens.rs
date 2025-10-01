use anyhow::{anyhow, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rand_core::{OsRng, RngCore};
use rsa::pkcs8::DecodePrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Row};
use uuid::Uuid;

pub struct TokenConfig {
    pub issuer: String,
    pub audience: String,
    pub access_ttl_seconds: i64,
    pub refresh_ttl_seconds: i64,
}

pub struct TokenSigner {
    pool: PgPool,
    config: TokenConfig,
    active_key: ActiveKey,
    fallback_jwk: Option<JwkKey>,
}

struct ActiveKey {
    kid: String,
    encoding_key: EncodingKey,
}

#[derive(Clone, Serialize)]
pub struct JwkKey {
    pub kty: &'static str,
    #[serde(rename = "use")]
    pub use_: &'static str,
    pub kid: String,
    pub alg: String,
    pub n: String,
    pub e: String,
}

pub struct TokenSubject {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub roles: Vec<String>,
}

#[derive(FromRow)]

#[derive(Debug, Clone)]
pub struct RefreshTokenAccount {
    pub jti: Uuid,
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_password_reset: Option<DateTime<Utc>>,
    pub force_password_reset: bool,
}
pub struct IssuedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at: DateTime<Utc>,
    pub refresh_expires_at: DateTime<Utc>,
    pub access_expires_in: i64,
    pub refresh_expires_in: i64,
    pub token_type: &'static str,
}

impl TokenSigner {
    pub async fn new(
        pool: PgPool,
        config: TokenConfig,
        fallback_private_pem: Option<&str>,
    ) -> Result<Self> {
        let db_key = Self::load_active_key(&pool).await?;

        let (active_key, fallback_jwk) = match db_key {
            Some(row) => {
                let encoding_key = EncodingKey::from_rsa_pem(row.private_pem.as_bytes())
                    .map_err(|err| anyhow!("Failed to parse active private key: {err}"))?;
                let jwk = JwkKey {
                    kty: "RSA",
                    use_: "sig",
                    kid: row.kid.clone(),
                    alg: row.alg.clone(),
                    n: row.n,
                    e: row.e,
                };
                (
                    ActiveKey {
                        kid: row.kid,
                        encoding_key,
                    },
                    Some(jwk),
                )
            }
            None => {
                let pem = fallback_private_pem
                    .ok_or_else(|| anyhow!("No signing key configured. Provide database key or JWT_DEV_PRIVATE_KEY_PEM"))?;
                let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes())
                    .map_err(|err| anyhow!("Failed to parse JWT_DEV_PRIVATE_KEY_PEM: {err}"))?;
                let (n, e) = Self::components_from_private_pem(pem)?;
                let jwk = JwkKey {
                    kty: "RSA",
                    use_: "sig",
                    kid: "local-dev".to_string(),
                    alg: "RS256".to_string(),
                    n,
                    e,
                };
                (
                    ActiveKey {
                        kid: jwk.kid.clone(),
                        encoding_key,
                    },
                    Some(jwk),
                )
            }
        };

        Ok(Self {
            pool,
            config,
            active_key,
            fallback_jwk,
        })
    }

    async fn load_active_key(pool: &PgPool) -> Result<Option<DbSigningKey>> {
        let row = sqlx::query(
            "SELECT kid, private_pem, alg, n, e FROM auth_signing_keys WHERE active = TRUE ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await?;

        if let Some(row) = row {
            let kid: String = row.try_get("kid")?;
            let private_pem: String = row.try_get("private_pem")?;
            let alg: String = row.try_get("alg")?;
            let n: String = row.try_get("n")?;
            let e: String = row.try_get("e")?;

            Ok(Some(DbSigningKey {
                kid,
                private_pem,
                alg,
                n,
                e,
            }))
        } else {
            Ok(None)
        }
    }

    fn components_from_private_pem(pem: &str) -> Result<(String, String)> {
        let private = RsaPrivateKey::from_pkcs8_pem(pem)
            .map_err(|err| anyhow!("Failed to parse RSA private key: {err}"))?;
        let public = private.to_public_key();
        let modulus = public.n().to_bytes_be();
        let exponent = public.e().to_bytes_be();
        let n = URL_SAFE_NO_PAD.encode(modulus);
        let e = URL_SAFE_NO_PAD.encode(exponent);
        Ok((n, e))
    }

    pub async fn jwks(&self) -> Result<Vec<JwkKey>> {
        let rows = sqlx::query(
            "SELECT kid, alg, n, e FROM auth_signing_keys WHERE active = TRUE ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut keys = Vec::new();
        for row in rows {
            let kid: String = row.try_get("kid")?;
            let alg: String = row.try_get("alg")?;
            let n: String = row.try_get("n")?;
            let e: String = row.try_get("e")?;
            keys.push(JwkKey {
                kty: "RSA",
                use_: "sig",
                kid,
                alg,
                n,
                e,
            });
        }

        if keys.is_empty() {
            if let Some(fallback) = &self.fallback_jwk {
                keys.push(fallback.clone());
            }
        }

        if keys.is_empty() {
            Err(anyhow!("No signing keys available for JWKS response"))
        } else {
            Ok(keys)
        }
    }

    pub async fn issue_tokens(&self, subject: TokenSubject) -> Result<IssuedTokens> {
        let now = Utc::now();
        let access_exp = now + Duration::seconds(self.config.access_ttl_seconds);
        let refresh_exp = now + Duration::seconds(self.config.refresh_ttl_seconds);

        let access_claims = AccessClaims {
            sub: subject.user_id.to_string(),
            tid: subject.tenant_id.to_string(),
            roles: &subject.roles,
            iss: &self.config.issuer,
            aud: &self.config.audience,
            exp: access_exp.timestamp(),
            iat: now.timestamp(),
            jti: Uuid::new_v4().to_string(),
        };

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.active_key.kid.clone());

        let access_token = encode(&header, &access_claims, &self.active_key.encoding_key)
            .map_err(|err| anyhow!("Failed to sign access token: {err}"))?;

        let refresh_token = Self::generate_refresh_token();
        let refresh_hash = Self::hash_refresh_token(&refresh_token);
        let refresh_jti = Uuid::new_v4();

        self.persist_refresh_token(refresh_jti, &subject, &refresh_hash, now, refresh_exp)
            .await?;

        Ok(IssuedTokens {
            access_token,
            refresh_token,
            access_expires_at: access_exp,
            refresh_expires_at: refresh_exp,
            access_expires_in: self.config.access_ttl_seconds,
            refresh_expires_in: self.config.refresh_ttl_seconds,
            token_type: "Bearer",
        })
    }

    fn generate_refresh_token() -> String {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let random = URL_SAFE_NO_PAD.encode(bytes);
        format!("{}.{}", Uuid::new_v4(), random)
    }

    fn hash_refresh_token(token: &str) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hasher.finalize().to_vec()
    }

    async fn persist_refresh_token(
        &self,
        jti: Uuid,
        subject: &TokenSubject,
        token_hash: &[u8],
        issued_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO auth_refresh_tokens (jti, user_id, tenant_id, token_hash, issued_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(jti)
        .bind(subject.user_id)
        .bind(subject.tenant_id)
        .bind(token_hash)
        .bind(issued_at)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|err| anyhow!("Failed to persist refresh token: {err}"))
    }

    pub async fn consume_refresh_token(&self, token: &str) -> Result<Option<RefreshTokenAccount>> {
        if token.trim().is_empty() {
            return Ok(None);
        }

        let hash = Self::hash_refresh_token(token);
        let mut tx = self.pool.begin().await?;
        // NOTE: We previously relied on a nullable revoked_at column to soft-revoke tokens.
        // Some environments (or older migrations) may lack that column, resulting in 500s during
        // /session refresh. To remain backward compatible we now perform an explicit SELECT then
        // DELETE (hard revoke) inside the same transaction. This preserves one-time semantics
        // without schema coupling.
        let row = sqlx::query!(
            r#"SELECT r.jti, r.user_id, r.tenant_id, r.expires_at,
                       u.name, u.email, u.role, u.is_active,
                       u.created_at, u.updated_at, u.last_password_reset, u.force_password_reset
                FROM auth_refresh_tokens r
                JOIN users u ON u.id = r.user_id
                WHERE r.token_hash = $1
                FOR UPDATE"#,
            hash.as_slice()
        )
        .fetch_optional(&mut *tx)
        .await?;

        let account = if let Some(row) = row {
            let now = Utc::now();
            // Hard delete regardless; token is single-use.
            sqlx::query!("DELETE FROM auth_refresh_tokens WHERE jti = $1", row.jti)
                .execute(&mut *tx)
                .await?;
            if row.expires_at <= now {
                None
            } else {
                Some(RefreshTokenAccount {
                    jti: row.jti,
                    user_id: row.user_id,
                    tenant_id: row.tenant_id,
                    name: row.name,
                    email: row.email,
                    role: row.role,
                    is_active: row.is_active,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                    last_password_reset: row.last_password_reset,
                    force_password_reset: row.force_password_reset,
                })
            }
        } else {
            None
        };

        tx.commit().await?;
        Ok(account)
    }
}

struct DbSigningKey {
    kid: String,
    private_pem: String,
    alg: String,
    n: String,
    e: String,
}

#[derive(Serialize)]
struct AccessClaims<'a> {
    sub: String,
    tid: String,
    roles: &'a [String],
    iss: &'a str,
    aud: &'a str,
    exp: i64,
    iat: i64,
    jti: String,
}
