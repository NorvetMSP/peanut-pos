use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::{header::AUTHORIZATION, request::Parts};

use crate::claims::Claims;
use crate::error::{AuthError, AuthResult};
use crate::verifier::JwtVerifier;

/// Extracts verified JWT claims from the request using the configured verifier.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub claims: Claims,
    pub token: String,
}

impl AuthContext {
    pub fn has_role(&self, role: &str) -> bool {
        self.claims.has_role(role)
    }

    pub fn into_claims(self) -> Claims {
        self.claims
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthContext
where
    Arc<JwtVerifier>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let verifier = Arc::<JwtVerifier>::from_ref(state);

        let header_value = parts
            .headers
            .get(AUTHORIZATION)
            .ok_or(AuthError::MissingAuthorization)?;

        let token = parse_bearer(header_value)?;
        let claims = verifier.verify(&token)?;

        Ok(Self { claims, token })
    }
}

fn parse_bearer(value: &axum::http::HeaderValue) -> AuthResult<String> {
    let raw = value
        .to_str()
        .map_err(|_| AuthError::InvalidAuthorization)?
        .trim();

    let token = raw
        .strip_prefix("Bearer ")
        .ok_or(AuthError::InvalidAuthorization)?
        .trim();

    if token.is_empty() {
        return Err(AuthError::InvalidAuthorization);
    }

    Ok(token.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn parse_bearer_accepts_valid_token() {
        let header = HeaderValue::from_static("Bearer abc.def.ghi");
        let token = parse_bearer(&header).expect("token");
        assert_eq!(token, "abc.def.ghi");
    }

    #[test]
    fn parse_bearer_rejects_wrong_scheme() {
        let header = HeaderValue::from_static("Basic credentials");
        let err = parse_bearer(&header).expect_err("should reject");
        assert!(matches!(err, AuthError::InvalidAuthorization));
    }

    #[test]
    fn parse_bearer_rejects_empty_value() {
        let header = HeaderValue::from_static("Bearer    ");
        let err = parse_bearer(&header).expect_err("should reject empty token");
        assert!(matches!(err, AuthError::InvalidAuthorization));
    }
}
