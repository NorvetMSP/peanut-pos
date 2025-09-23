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
