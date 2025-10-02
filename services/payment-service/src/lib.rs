use std::sync::Arc;
use common_auth::JwtVerifier;
use axum::extract::FromRef;


#[derive(Clone)]
pub struct AppState {
    pub jwt_verifier: Arc<JwtVerifier>,
}

pub mod payment_handlers;
impl FromRef<AppState> for Arc<JwtVerifier> { fn from_ref(state:&AppState)->Self { state.jwt_verifier.clone() } }
