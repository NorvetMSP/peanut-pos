use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde_json::Value;
use tracing::debug;

use crate::claims::Claims;
use crate::config::JwtConfig;
use crate::error::{AuthError, AuthResult};
use crate::jwks::JwksFetcher;

/// Thread-safe store for decoding keys loaded from JWKS/PEM sources.
#[derive(Clone, Default)]
pub struct InMemoryKeyStore {
    inner: Arc<RwLock<HashMap<String, DecodingKey>>>,
}

impl InMemoryKeyStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_key(&self, kid: impl Into<String>, key: DecodingKey) {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        guard.insert(kid.into(), key);
    }

    pub fn insert_rsa_pem(&self, kid: impl Into<String>, pem: &[u8]) -> AuthResult<()> {
        let kid = kid.into();
        let key = DecodingKey::from_rsa_pem(pem)
            .map_err(|err| AuthError::KeyParse(kid.clone(), err.to_string()))?;
        self.insert_key(kid, key);
        Ok(())
    }

    pub fn get(&self, kid: &str) -> Option<DecodingKey> {
        let guard = self.inner.read().expect("rwlock poisoned");
        guard.get(kid).cloned()
    }

    pub fn contains(&self, kid: &str) -> bool {
        let guard = self.inner.read().expect("rwlock poisoned");
        guard.contains_key(kid)
    }

    pub fn replace_all<I>(&self, entries: I)
    where
        I: IntoIterator<Item = (String, DecodingKey)>,
    {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        guard.clear();
        for (kid, key) in entries.into_iter() {
            guard.insert(kid, key);
        }
    }
}

#[derive(Clone)]
pub struct JwtVerifier {
    config: JwtConfig,
    store: InMemoryKeyStore,
    jwks: Option<JwksFetcher>,
}

impl JwtVerifier {
    pub fn new(config: JwtConfig) -> Self {
        Self {
            config,
            store: InMemoryKeyStore::new(),
            jwks: None,
        }
    }

    pub fn with_store(config: JwtConfig, store: InMemoryKeyStore) -> Self {
        Self {
            config,
            store,
            jwks: None,
        }
    }

    pub fn builder(config: JwtConfig) -> JwtVerifierBuilder {
        JwtVerifierBuilder::new(config)
    }

    pub fn config(&self) -> &JwtConfig {
        &self.config
    }

    pub fn store(&self) -> &InMemoryKeyStore {
        &self.store
    }

    pub fn jwks_fetcher(&self) -> Option<&JwksFetcher> {
        self.jwks.as_ref()
    }

    pub fn verify(&self, token: &str) -> AuthResult<Claims> {
        let header =
            decode_header(token).map_err(|err| AuthError::InvalidHeader(err.to_string()))?;
        let kid = header.kid.ok_or(AuthError::MissingKeyId)?;
        let key = self
            .store
            .get(&kid)
            .ok_or_else(|| AuthError::UnknownKeyId(kid.clone()))?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[self.config.issuer.clone()]);
        validation.set_audience(&[self.config.audience.clone()]);
        validation.leeway = self.config.leeway_seconds.into();

        let token_data = decode::<Value>(token, &key, &validation)?;
        let claims = Claims::try_from(token_data.claims)?;
        debug!(kid, "verified JWT successfully");
        Ok(claims)
    }

    pub async fn refresh_jwks(&self) -> AuthResult<usize> {
        let fetcher = match &self.jwks {
            Some(fetcher) => fetcher,
            None => return Ok(0),
        };

        let keys = fetcher.fetch().await?;
        let count = keys.len();
        if count > 0 {
            self.store.replace_all(keys);
        }
        Ok(count)
    }
}

pub struct JwtVerifierBuilder {
    config: JwtConfig,
    store: InMemoryKeyStore,
    jwks: Option<JwksFetcher>,
}

impl JwtVerifierBuilder {
    fn new(config: JwtConfig) -> Self {
        Self {
            config,
            store: InMemoryKeyStore::new(),
            jwks: None,
        }
    }

    pub fn with_store(mut self, store: InMemoryKeyStore) -> Self {
        self.store = store;
        self
    }

    pub fn with_decoding_key(self, kid: impl Into<String>, key: DecodingKey) -> Self {
        self.store.insert_key(kid, key);
        self
    }

    pub fn with_rsa_pem(self, kid: impl Into<String>, pem: &[u8]) -> AuthResult<Self> {
        self.store.insert_rsa_pem(kid, pem)?;
        Ok(self)
    }

    pub fn with_jwks_url(mut self, url: impl Into<String>) -> Self {
        self.jwks = Some(JwksFetcher::new(url));
        self
    }

    pub fn with_jwks_fetcher(mut self, fetcher: JwksFetcher) -> Self {
        self.jwks = Some(fetcher);
        self
    }

    pub async fn build(self) -> AuthResult<JwtVerifier> {
        let verifier = JwtVerifier {
            config: self.config,
            store: self.store,
            jwks: self.jwks,
        };

        if verifier.jwks.is_some() {
            verifier.refresh_jwks().await?;
        }

        Ok(verifier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use chrono::Utc;
    use httpmock::prelude::*;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use rsa::pkcs1::{EncodeRsaPrivateKey, EncodeRsaPublicKey, LineEnding};
    use rsa::rand_core::OsRng;
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;
    use serde::Serialize;
    use uuid::Uuid;

    #[derive(Serialize)]
    struct TokenClaims<'a> {
        sub: &'a str,
        tid: &'a str,
        roles: &'a [String],
        iss: &'a str,
        aud: &'a str,
        exp: i64,
        iat: i64,
    }

    struct KeyMaterial {
        encoding: EncodingKey,
        decoding: DecodingKey,
        modulus: String,
        exponent: String,
    }

    fn generate_key_material() -> KeyMaterial {
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("key generation");
        let public_key = private_key.to_public_key();

        let private_pem = private_key
            .to_pkcs1_pem(LineEnding::LF)
            .expect("private pem");
        let public_pem = public_key.to_pkcs1_pem(LineEnding::LF).expect("public pem");

        let encoding = EncodingKey::from_rsa_pem(private_pem.as_bytes()).expect("encoding key");
        let decoding = DecodingKey::from_rsa_pem(public_pem.as_bytes()).expect("decoding key");
        let modulus = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
        let exponent = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());

        KeyMaterial {
            encoding,
            decoding,
            modulus,
            exponent,
        }
    }

    fn issue_token(
        encoding: &EncodingKey,
        kid: &str,
        issuer: &str,
        audience: &str,
    ) -> (String, Uuid, Uuid, Vec<String>) {
        let subject = Uuid::new_v4();
        let tenant = Uuid::new_v4();
        let issued_at = Utc::now().timestamp();
        let expires_at = issued_at + 600;
        let roles = vec!["admin".to_string(), "user".to_string()];
        let subject_str = subject.to_string();
        let tenant_str = tenant.to_string();

        let claims = TokenClaims {
            sub: &subject_str,
            tid: &tenant_str,
            roles: &roles,
            iss: issuer,
            aud: audience,
            exp: expires_at,
            iat: issued_at,
        };

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        let token = encode(&header, &claims, encoding).expect("sign token");

        (token, subject, tenant, roles)
    }

    #[test]
    fn key_store_insert_replace_round_trip() {
        let store = InMemoryKeyStore::new();
        assert!(!store.contains("kid"));
        store.insert_key("kid", DecodingKey::from_secret(b"secret"));
        assert!(store.contains("kid"));
        assert!(store.get("kid").is_some());

        store.replace_all(vec![(
            "another".to_string(),
            DecodingKey::from_secret(b"other"),
        )]);
        assert!(!store.contains("kid"));
        assert!(store.contains("another"));
    }

    #[test]
    fn verifier_accepts_valid_token() {
        let material = generate_key_material();
        let kid = "test-key";
        let config = JwtConfig::new("test-issuer", "test-audience");
        let store = InMemoryKeyStore::new();
        store.insert_key(kid, material.decoding.clone());
        let verifier = JwtVerifier::with_store(config, store);

        let (token, subject, tenant, roles) =
            issue_token(&material.encoding, kid, "test-issuer", "test-audience");
        let claims = verifier.verify(&token).expect("verification succeeds");

        assert_eq!(claims.subject, subject);
        assert_eq!(claims.tenant_id, tenant);
        assert_eq!(claims.roles, roles);
        assert_eq!(claims.issuer, "test-issuer");
        assert_eq!(claims.audience, vec!["test-audience".to_string()]);
    }

    #[test]
    fn verifier_rejects_unknown_kid() {
        let material = generate_key_material();
        let kid = "missing";
        let config = JwtConfig::new("issuer", "aud");
        let store = InMemoryKeyStore::new();
        let verifier = JwtVerifier::with_store(config, store);

        let (token, _, _, _) = issue_token(&material.encoding, kid, "issuer", "aud");
        let err = verifier
            .verify(&token)
            .expect_err("verification should fail");
        match err {
            AuthError::UnknownKeyId(actual) => assert_eq!(actual, kid),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn refresh_jwks_updates_store() {
        let material = generate_key_material();
        let server = MockServer::start();
        let kid = "fetched-key";
        let body = serde_json::json!({
            "keys": [
                {
                    "kid": kid,
                    "kty": "RSA",
                    "alg": "RS256",
                    "n": material.modulus,
                    "e": material.exponent
                }
            ]
        });

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/jwks");
            then.status(200)
                .header("content-type", "application/json")
                .body(body.to_string());
        });

        let config = JwtConfig::new("issuer", "audience");
        let store = InMemoryKeyStore::new();
        let verifier = JwtVerifier {
            config,
            store,
            jwks: Some(JwksFetcher::new(format!("{}/jwks", server.base_url()))),
        };

        assert!(!verifier.store().contains(kid));
        let refreshed = verifier.refresh_jwks().await.expect("refresh succeeds");
        assert_eq!(refreshed, 1);
        assert!(verifier.store().contains(kid));
    }

    #[tokio::test]
    async fn refresh_jwks_without_fetcher_returns_zero() {
        let config = JwtConfig::new("issuer", "audience");
        let store = InMemoryKeyStore::new();
        let verifier = JwtVerifier::with_store(config, store);

        let refreshed = verifier.refresh_jwks().await.expect("refresh succeeds");
        assert_eq!(refreshed, 0);
    }
}
