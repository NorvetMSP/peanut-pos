/// Runtime configuration for JWT verification.
#[derive(Debug, Clone)]
pub struct JwtConfig {
    /// Expected issuer claim (iss).
    pub issuer: String,
    /// Expected audience claim (aud).
    pub audience: String,
    /// Allowable clock skew in seconds when validating exp/nbf.
    pub leeway_seconds: u32,
}

impl JwtConfig {
    /// Construct config with sensible defaults (30 second leeway).
    pub fn new(issuer: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            issuer: issuer.into(),
            audience: audience.into(),
            leeway_seconds: 30,
        }
    }

    /// Adjust the allowed leeway.
    pub fn with_leeway(mut self, seconds: u32) -> Self {
        self.leeway_seconds = seconds;
        self
    }
}
