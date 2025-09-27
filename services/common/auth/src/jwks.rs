use jsonwebtoken::DecodingKey;
use reqwest::Client;
use serde::Deserialize;

use crate::error::{AuthError, AuthResult};

#[derive(Clone)]
pub struct JwksFetcher {
    client: Client,
    url: String,
}

impl JwksFetcher {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            url: url.into(),
        }
    }

    pub fn with_client(client: Client, url: impl Into<String>) -> Self {
        Self {
            client,
            url: url.into(),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub async fn fetch(&self) -> AuthResult<Vec<(String, DecodingKey)>> {
        let response = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(|err| AuthError::JwksFetch(err.to_string()))?;

        if !response.status().is_success() {
            return Err(AuthError::JwksFetch(format!(
                "HTTP {} from {}",
                response.status(),
                self.url
            )));
        }

        let body: JwksResponse = response
            .json()
            .await
            .map_err(|err| AuthError::JwksDecode(err.to_string()))?;

        let mut keys = Vec::new();
        for key in body.keys.into_iter() {
            let kid = key.kid.ok_or(AuthError::JwksMissingKid)?;
            let kty = key.kty.unwrap_or_else(|| "RSA".to_string());
            if kty != "RSA" {
                return Err(AuthError::JwksUnsupportedKey { kid, kty });
            }

            if let Some(alg) = key.alg {
                if alg != "RS256" {
                    return Err(AuthError::JwksUnsupportedAlg { kid, alg });
                }
            }

            let modulus = key
                .n
                .ok_or_else(|| AuthError::JwksMissingComponents(kid.clone()))?;
            let exponent = key
                .e
                .ok_or_else(|| AuthError::JwksMissingComponents(kid.clone()))?;

            let decoding_key = DecodingKey::from_rsa_components(&modulus, &exponent)
                .map_err(|err| AuthError::KeyParse(kid.clone(), err.to_string()))?;
            keys.push((kid, decoding_key));
        }

        Ok(keys)
    }
}

#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<JwkEntry>,
}

#[derive(Debug, Deserialize)]
struct JwkEntry {
    kid: Option<String>,
    kty: Option<String>,
    alg: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use httpmock::prelude::*;
    use rsa::rand_core::OsRng;
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;

    fn sample_components() -> (String, String) {
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("key generation");
        let public_key = private_key.to_public_key();
        let modulus = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
        let exponent = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());
        (modulus, exponent)
    }

    #[tokio::test]
    async fn fetch_parses_valid_response() {
        let (modulus, exponent) = sample_components();
        let server = MockServer::start();
        let body = serde_json::json!({
            "keys": [
                {
                    "kid": "key-1",
                    "kty": "RSA",
                    "alg": "RS256",
                    "n": modulus,
                    "e": exponent
                }
            ]
        });

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/jwks");
            then.status(200)
                .header("content-type", "application/json")
                .body(body.to_string());
        });

        let fetcher = JwksFetcher::new(format!("{}/jwks", server.base_url()));
        let keys = fetcher.fetch().await.expect("fetch succeeds");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].0, "key-1");
    }

    #[tokio::test]
    async fn fetch_rejects_error_status() {
        let server = MockServer::start();
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/jwks");
            then.status(502);
        });

        let fetcher = JwksFetcher::new(format!("{}/jwks", server.base_url()));
        let err = match fetcher.fetch().await {
            Ok(_) => panic!("fetch should fail"),
            Err(err) => err,
        };
        match err {
            AuthError::JwksFetch(message) => assert!(message.contains("502")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_requires_kid() {
        let (modulus, exponent) = sample_components();
        let server = MockServer::start();
        let body = serde_json::json!({
            "keys": [
                {
                    "kty": "RSA",
                    "alg": "RS256",
                    "n": modulus,
                    "e": exponent
                }
            ]
        });

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/jwks");
            then.status(200)
                .header("content-type", "application/json")
                .body(body.to_string());
        });

        let fetcher = JwksFetcher::new(format!("{}/jwks", server.base_url()));
        let err = match fetcher.fetch().await {
            Ok(_) => panic!("fetch should fail"),
            Err(err) => err,
        };
        assert!(matches!(err, AuthError::JwksMissingKid));
    }
    #[tokio::test]
    async fn fetch_rejects_unsupported_key_type() {
        let (modulus, exponent) = sample_components();
        let server = MockServer::start();
        let body = serde_json::json!({
            "keys": [
                {
                    "kid": "weird",
                    "kty": "EC",
                    "alg": "RS256",
                    "n": modulus,
                    "e": exponent
                }
            ]
        });

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/jwks");
            then.status(200)
                .header("content-type", "application/json")
                .body(body.to_string());
        });

        let fetcher = JwksFetcher::new(format!("{}/jwks", server.base_url()));
        let err = match fetcher.fetch().await {
            Ok(_) => panic!("fetch should fail"),
            Err(err) => err,
        };
        assert!(matches!(err, AuthError::JwksUnsupportedKey { .. }));
    }

    #[tokio::test]
    async fn fetch_rejects_unsupported_algorithm() {
        let (modulus, exponent) = sample_components();
        let server = MockServer::start();
        let body = serde_json::json!({
            "keys": [
                {
                    "kid": "bad-alg",
                    "kty": "RSA",
                    "alg": "HS256",
                    "n": modulus,
                    "e": exponent
                }
            ]
        });

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/jwks");
            then.status(200)
                .header("content-type", "application/json")
                .body(body.to_string());
        });

        let fetcher = JwksFetcher::new(format!("{}/jwks", server.base_url()));
        let err = match fetcher.fetch().await {
            Ok(_) => panic!("fetch should fail"),
            Err(err) => err,
        };
        assert!(matches!(err, AuthError::JwksUnsupportedAlg { .. }));
    }

    #[tokio::test]
    async fn fetch_rejects_missing_modulus_or_exponent() {
        let server = MockServer::start();
        let body = serde_json::json!({
            "keys": [
                {
                    "kid": "incomplete",
                    "kty": "RSA",
                    "alg": "RS256",
                    "e": "AQAB"
                }
            ]
        });

        let _mock = server.mock(|when, then| {
            when.method(GET).path("/jwks");
            then.status(200)
                .header("content-type", "application/json")
                .body(body.to_string());
        });

        let fetcher = JwksFetcher::new(format!("{}/jwks", server.base_url()));
        let err = match fetcher.fetch().await {
            Ok(_) => panic!("fetch should fail"),
            Err(err) => err,
        };
        assert!(matches!(err, AuthError::JwksMissingComponents(_)));
    }
}
