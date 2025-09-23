use anyhow::{anyhow, Context, Result};
use std::collections::HashSet;
use std::env;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieSameSite {
    Lax,
    Strict,
    None,
}

impl CookieSameSite {
    pub fn as_str(&self) -> &'static str {
        match self {
            CookieSameSite::Lax => "Lax",
            CookieSameSite::Strict => "Strict",
            CookieSameSite::None => "None",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub require_mfa: bool,
    pub required_roles: HashSet<String>,
    pub bypass_tenants: HashSet<Uuid>,
    pub mfa_issuer: String,
    pub mfa_activity_topic: String,
    pub suspicious_webhook_url: Option<String>,
    pub suspicious_webhook_bearer: Option<String>,
    pub refresh_cookie_name: String,
    pub refresh_cookie_domain: Option<String>,
    pub refresh_cookie_secure: bool,
    pub refresh_cookie_same_site: CookieSameSite,
}

impl AuthConfig {
    pub fn should_enforce_for(&self, role: &str, tenant_id: Uuid, has_secret: bool) -> bool {
        if self.bypass_tenants.contains(&tenant_id) {
            // If a tenant is explicitly bypassed we still require MFA when a secret exists
            // so we do not silently disable previously-enrolled users.
            return has_secret;
        }

        if has_secret {
            return true;
        }

        if self.require_mfa {
            return true;
        }

        let role_key = role.to_ascii_lowercase();
        self.required_roles.contains(&role_key)
    }

    pub fn required_roles_sorted(&self) -> Vec<String> {
        let mut roles = self.required_roles.iter().cloned().collect::<Vec<_>>();
        roles.sort();
        roles
    }

    pub fn bypass_tenants_sorted(&self) -> Vec<Uuid> {
        let mut tenants = self.bypass_tenants.iter().cloned().collect::<Vec<_>>();
        tenants.sort();
        tenants
    }
}

pub fn load_auth_config() -> Result<AuthConfig> {
    let require_mfa = bool_from_env("AUTH_REQUIRE_MFA").unwrap_or(false);

    let required_roles = env::var("AUTH_MFA_REQUIRED_ROLES")
        .ok()
        .map(|value| parse_roles(&value))
        .unwrap_or_else(default_roles);

    let bypass_tenants = env::var("AUTH_MFA_BYPASS_TENANTS")
        .ok()
        .map(|value| parse_tenant_list(&value))
        .transpose()
        .context("Failed to parse AUTH_MFA_BYPASS_TENANTS")?
        .unwrap_or_default();

    let mfa_issuer = env::var("AUTH_MFA_ISSUER").unwrap_or_else(|_| "NovaPOS".to_string());

    let mfa_activity_topic = env::var("SECURITY_MFA_ACTIVITY_TOPIC")
        .unwrap_or_else(|_| "security.mfa.activity".to_string());

    let suspicious_webhook_url = env::var("SECURITY_SUSPICIOUS_WEBHOOK_URL")
        .ok()
        .and_then(|value| normalize_optional(&value));
    let suspicious_webhook_bearer = env::var("SECURITY_SUSPICIOUS_WEBHOOK_BEARER")
        .ok()
        .and_then(|value| normalize_optional(&value));

    let refresh_cookie_name =
        env::var("AUTH_REFRESH_COOKIE_NAME").unwrap_or_else(|_| "novapos_refresh".to_string());
    let refresh_cookie_domain = env::var("AUTH_REFRESH_COOKIE_DOMAIN")
        .ok()
        .and_then(|value| normalize_optional(&value));
    let refresh_cookie_secure = bool_from_env("AUTH_REFRESH_COOKIE_SECURE").unwrap_or(false);
    let refresh_cookie_same_site = env::var("AUTH_REFRESH_COOKIE_SAMESITE")
        .ok()
        .map(|value| parse_same_site(&value))
        .transpose()
        .context("Failed to parse AUTH_REFRESH_COOKIE_SAMESITE")?
        .unwrap_or(CookieSameSite::Lax);

    Ok(AuthConfig {
        require_mfa,
        required_roles,
        bypass_tenants,
        mfa_issuer,
        mfa_activity_topic,
        suspicious_webhook_url,
        suspicious_webhook_bearer,
        refresh_cookie_name,
        refresh_cookie_domain,
        refresh_cookie_secure,
        refresh_cookie_same_site,
    })
}

fn bool_from_env(key: &str) -> Option<bool> {
    env::var(key).ok().map(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn parse_roles(value: &str) -> HashSet<String> {
    value
        .split(|c| c == ',' || c == ';' || c == ' ')
        .filter_map(|item| {
            let role = item.trim();
            if role.is_empty() {
                None
            } else {
                Some(role.to_ascii_lowercase())
            }
        })
        .collect()
}

fn default_roles() -> HashSet<String> {
    HashSet::from([
        "super_admin".to_string(),
        "admin".to_string(),
        "manager".to_string(),
    ])
}

fn parse_tenant_list(value: &str) -> Result<HashSet<Uuid>> {
    let mut tenants = HashSet::new();
    for item in value.split(|c| c == ',' || c == ';' || c == ' ') {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tenant = Uuid::parse_str(trimmed)
            .map_err(|err| anyhow!("Invalid tenant UUID '{trimmed}': {err}"))?;
        tenants.insert(tenant);
    }
    Ok(tenants)
}

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_same_site(value: &str) -> Result<CookieSameSite> {
    match value.trim().to_ascii_lowercase().as_str() {
        "lax" => Ok(CookieSameSite::Lax),
        "strict" => Ok(CookieSameSite::Strict),
        "none" => Ok(CookieSameSite::None),
        other => Err(anyhow!(
            "Unsupported cookie same-site policy '{other}'. Use Lax, Strict, or None."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_from_env_parses() {
        std::env::set_var("TEST_BOOL_TRUE", "true");
        std::env::set_var("TEST_BOOL_ONE", "1");
        std::env::set_var("TEST_BOOL_FALSE", "no");
        assert_eq!(bool_from_env("TEST_BOOL_TRUE"), Some(true));
        assert_eq!(bool_from_env("TEST_BOOL_ONE"), Some(true));
        assert_eq!(bool_from_env("TEST_BOOL_FALSE"), Some(false));
    }

    #[test]
    fn parse_roles_normalises() {
        let roles = parse_roles("Admin,super_admin Manager");
        assert!(roles.contains("admin"));
        assert!(roles.contains("super_admin"));
        assert!(roles.contains("manager"));
        assert!(!roles.contains("Admin"));
    }
}
