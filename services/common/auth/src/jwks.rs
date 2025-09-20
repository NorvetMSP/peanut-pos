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
