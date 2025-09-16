use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Shared payment.completed event payload produced by the gateway.
#[derive(Debug, Serialize, Deserialize)]
pub struct PaymentCompletedEvent {
    pub order_id: Uuid,
    pub tenant_id: Uuid,
    pub method: String,
    pub amount: f64,
}
