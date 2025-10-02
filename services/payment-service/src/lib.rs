use std::sync::Arc;
use common_auth::JwtVerifier;
use common_security::roles::Role;
use axum::extract::FromRef;

pub const PAYMENT_ROLES: &[Role] = &[Role::Admin, Role::Manager, Role::Inventory]; // Temporary mapping until expanded role model (POL-1)

#[derive(Clone)]
pub struct AppState {
    pub jwt_verifier: Arc<JwtVerifier>,
}

pub mod payment_handlers;
impl FromRef<AppState> for Arc<JwtVerifier> { fn from_ref(state:&AppState)->Self { state.jwt_verifier.clone() } }
