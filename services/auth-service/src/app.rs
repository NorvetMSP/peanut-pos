use std::sync::Arc;

use axum::extract::FromRef;
use common_auth::JwtVerifier;
use reqwest::Client;
use sqlx::PgPool;
use tracing::warn;

use crate::config::AuthConfig;
use crate::metrics::AuthMetrics;
use crate::notifications::{
    post_suspicious_webhook, publish_mfa_activity, KafkaProducer, MfaActivityEvent,
    SuspiciousLoginPayload,
};
use crate::tokens::TokenSigner;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt_verifier: Arc<JwtVerifier>,
    pub token_signer: Arc<TokenSigner>,
    pub config: Arc<AuthConfig>,
    pub kafka_producer: Arc<dyn KafkaProducer>,
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
        let mut kafka_sent = false;
        for attempt in 0..=1 {
            match publish_mfa_activity(
                self.kafka_producer.as_ref(),
                &self.config.mfa_activity_topic,
                &event,
            )
            .await
            {
                Ok(_) => {
                    kafka_sent = true;
                    break;
                }
                Err(err) => {
                    warn!(
                        attempt,
                        ?err,
                        tenant_id = %event.tenant_id,
                        trace_id = %event.trace_id,
                        "Failed to publish MFA activity",
                    );
                }
            }
        }

        if !kafka_sent {
            if let Some(dlq) = &self.config.mfa_dead_letter_topic {
                match serde_json::to_string(&event) {
                    Ok(payload) => {
                        if let Err(err) = self
                            .kafka_producer
                            .send(dlq, &event.tenant_id.to_string(), payload)
                            .await
                        {
                            warn!(
                                ?err,
                                tenant_id = %event.tenant_id,
                                trace_id = %event.trace_id,
                                "Failed to publish MFA activity to DLQ",
                            );
                        }
                    }
                    Err(err) => {
                        warn!(
                            ?err,
                            tenant_id = %event.tenant_id,
                            trace_id = %event.trace_id,
                            "Failed to serialise MFA event for DLQ",
                        );
                    }
                }
            }
        }

        if let Some(message) = webhook_message {
            if let Some(url) = &self.config.suspicious_webhook_url {
                if !url.is_empty() {
                    let bearer = self.config.suspicious_webhook_bearer.as_deref();
                    let payload = SuspiciousLoginPayload { text: message };
                    for attempt in 0..=1 {
                        match post_suspicious_webhook(&self.http_client, url, bearer, &payload)
                            .await
                        {
                            Ok(_) => break,
                            Err(err) => {
                                warn!(attempt, ?err, trace_id = %event.trace_id, "Failed to post suspicious login webhook");
                                if attempt == 1 {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
