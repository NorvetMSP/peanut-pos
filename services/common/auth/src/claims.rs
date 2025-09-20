use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AuthError, AuthResult};

/// Application-focused representation of verified JWT claims.
#[derive(Debug, Clone, Serialize)]
pub struct Claims {
    pub subject: Uuid,
    pub tenant_id: Uuid,
    pub roles: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub issued_at: Option<DateTime<Utc>>,
    pub issuer: String,
    pub audience: Vec<String>,
    pub raw: serde_json::Value,
}

impl Claims {
    /// Convenience helper for role checks.
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|value| value == role)
    }
}

#[derive(Debug, Deserialize)]
struct ClaimsRepr {
    sub: String,
    #[serde(rename = "tid")]
    tenant_id: String,
    #[serde(default)]
    roles: Vec<String>,
    exp: i64,
    #[serde(default)]
    iat: Option<i64>,
    iss: String,
    #[serde(default)]
    aud: Option<AudienceRepr>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AudienceRepr {
    Single(String),
    Many(Vec<String>),
}

impl TryFrom<ClaimsRepr> for Claims {
    type Error = AuthError;

    fn try_from(value: ClaimsRepr) -> AuthResult<Self> {
        let subject = Uuid::parse_str(&value.sub)
            .map_err(|_| AuthError::InvalidClaim("sub", value.sub.clone()))?;
        let tenant_id = Uuid::parse_str(&value.tenant_id)
            .map_err(|_| AuthError::InvalidClaim("tid", value.tenant_id.clone()))?;

        let expires_at = Utc
            .timestamp_opt(value.exp, 0)
            .single()
            .ok_or_else(|| AuthError::InvalidClaim("exp", value.exp.to_string()))?;

        let issued_at = match value.iat {
            Some(iat) => Some(
                Utc.timestamp_opt(iat, 0)
                    .single()
                    .ok_or_else(|| AuthError::InvalidClaim("iat", iat.to_string()))?,
            ),
            None => None,
        };

        let audience = match value.aud {
            Some(AudienceRepr::Single(item)) => vec![item],
            Some(AudienceRepr::Many(items)) => items,
            None => Vec::new(),
        };

        Ok(Self {
            subject,
            tenant_id,
            roles: value.roles,
            expires_at,
            issued_at,
            issuer: value.iss,
            audience,
            raw: serde_json::Value::Null,
        })
    }
}

impl TryFrom<serde_json::Value> for Claims {
    type Error = AuthError;

    fn try_from(value: serde_json::Value) -> AuthResult<Self> {
        let repr: ClaimsRepr = serde_json::from_value(value.clone())
            .map_err(|err| AuthError::InvalidJson(err.to_string()))?;
        let mut claims = Claims::try_from(repr)?;
        claims.raw = value;
        Ok(claims)
    }
}
