pub mod claims;
pub mod config;
pub mod error;
pub mod extractors;
pub mod guards;
pub mod jwks;
pub mod roles;
pub mod verifier;

pub use claims::Claims;
pub use config::JwtConfig;
pub use error::{AuthError, AuthResult};
pub use extractors::AuthContext;
pub use guards::{ensure_role, tenant_id_from_request, GuardError};
pub use jwks::JwksFetcher;
pub use roles::{ROLE_ADMIN, ROLE_CASHIER, ROLE_HIERARCHY, ROLE_MANAGER, ROLE_SUPER_ADMIN};
pub use verifier::{InMemoryKeyStore, JwtVerifier, JwtVerifierBuilder};
