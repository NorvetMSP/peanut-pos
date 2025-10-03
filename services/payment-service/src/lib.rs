use std::sync::Arc;
use common_auth::JwtVerifier;
use axum::extract::FromRef;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use common_audit::{BufferedAuditProducer, KafkaAuditSink};


#[derive(Clone)]
pub struct AppState {
    pub jwt_verifier: Arc<JwtVerifier>,
    #[cfg(any(feature = "kafka", feature = "kafka-producer"))] pub audit_producer: Option<Arc<BufferedAuditProducer<KafkaAuditSink>>>,
}

pub mod payment_handlers;
impl FromRef<AppState> for Arc<JwtVerifier> { fn from_ref(state:&AppState)->Self { state.jwt_verifier.clone() } }
