use std::sync::Arc;
use common_auth::{JwtVerifier, ROLE_ADMIN, ROLE_CASHIER, ROLE_SUPER_ADMIN};
use axum::extract::FromRef;

pub const PAYMENT_ROLES: &[&str] = &[ROLE_SUPER_ADMIN, ROLE_ADMIN, ROLE_CASHIER];

#[derive(Clone)]
pub struct AppState {
    pub jwt_verifier: Arc<JwtVerifier>,
}

pub mod payment_handlers;
impl FromRef<AppState> for Arc<JwtVerifier> { fn from_ref(state:&AppState)->Self { state.jwt_verifier.clone() } }
