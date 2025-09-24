use std::sync::Arc;

use axum::extract::FromRef;
use common_auth::JwtVerifier;
use rdkafka::producer::FutureProducer;
use reqwest::Client;
use sqlx::PgPool;
use tracing::warn;

use crate::config::AuthConfig;
use crate::metrics::AuthMetrics;
use crate::notifications::{
    post_suspicious_webhook, publish_mfa_activity, MfaActivityEvent, SuspiciousLoginPayload,
};
use crate::tokens::TokenSigner;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt_verifier: Arc<JwtVerifier>,
    pub token_signer: Arc<TokenSigner>,
    pub config: Arc<AuthConfig>,
    pub kafka_producer: FutureProducer,
    pub http_client: Client,
    pub metrics: Arc<AuthMetrics>,
}

impl FromRef<AppState> for Arc<JwtVerifier> {
    fn from_ref(state: &AppState) -> Self {
        state.jwt_verifier.clone()
    }
}

impl FromRef<AppState> for Arc<TokenSigner> {
    fn from_ref(state: &AppState) -> Self {
        state.token_signer.clone()
    }
}

impl FromRef<AppState> for Arc<AuthConfig> {
    fn from_ref(state: &AppState) -> Self {
        state.config.clone()
    }
}

impl AppState {
    pub fn record_login_metric(&self, outcome: &str) {
        self.metrics.login_attempt(outcome);
    }

    pub fn record_mfa_metric(&self, event: &str) {
        self.metrics.mfa_event(event);
    }

    pub async fn emit_mfa_activity(
        &self,
        event: MfaActivityEvent,
        webhook_message: Option<String>,
    ) {
        if let Err(err) = publish_mfa_activity(
            &self.kafka_producer,
            &self.config.mfa_activity_topic,
            &event,
        )
        .await
        {
            warn!(
                ?err,
                tenant_id = %event.tenant_id,
                trace_id = %event.trace_id,
                "Failed to publish MFA activity"
            );
        }

        if let Some(message) = webhook_message {
            if let Some(url) = &self.config.suspicious_webhook_url {
                if !url.is_empty() {
                    let bearer = self.config.suspicious_webhook_bearer.as_deref();
                    let payload = SuspiciousLoginPayload { text: message };
                    if let Err(err) =
                        post_suspicious_webhook(&self.http_client, url, bearer, &payload).await
                    {
                        warn!(
                            ?err,
                            trace_id = %event.trace_id,
                            "Failed to post suspicious login webhook"
                        );
                    }
                }
            }
        }
    }
}
