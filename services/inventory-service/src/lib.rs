pub mod inventory_handlers;
pub mod reservation_handlers;
pub mod location_handlers;
mod main_impl_placeholder {} // placeholder to avoid pulling main
pub use crate::inventory_handlers::*;
pub use crate::reservation_handlers::*;
pub use crate::location_handlers::*;
pub const DEFAULT_THRESHOLD: i32 = 5;
/// Returns true if inventory crossed from above threshold to at/below threshold.
///
/// Crossing logic:
/// - Emit when `prev > threshold` and `new <= threshold`.
/// - Do NOT emit repeatedly when remaining below threshold (prevents spam).
/// - Do NOT emit when increasing stock.
#[inline]
pub fn crossed_below_threshold(prev: i32, new: i32, threshold: i32) -> bool {
	prev > threshold && new <= threshold
}
// Minimal AppState mirror for tests (does not spawn consumer logic)
use std::sync::Arc;
use sqlx::PgPool;
use common_auth::JwtVerifier;
#[cfg(any(feature = "kafka", feature = "kafka-producer"))] use rdkafka::producer::FutureProducer;
use common_observability::InventoryMetrics;
use std::time::Duration;
#[derive(Clone)]
pub struct AppState {
	pub db: PgPool,
	pub jwt_verifier: Arc<JwtVerifier>,
	pub multi_location_enabled: bool,
	pub reservation_default_ttl: Duration,
	pub reservation_expiry_sweep: Duration,
	pub dual_write_enabled: bool,
	#[cfg(any(feature = "kafka", feature = "kafka-producer"))] pub kafka_producer: FutureProducer,
	pub metrics: Arc<InventoryMetrics>,
}

#[cfg(not(any(feature = "kafka", feature = "kafka-producer")))]
impl AppState {
	pub fn dummy_kafka_producer() {}
}

#[cfg(test)]
mod tests {
	use super::crossed_below_threshold;

	#[test]
	fn emits_when_crossing_from_above_to_equal() {
		assert!(crossed_below_threshold(10, 5, 5));
	}

	#[test]
	fn emits_when_crossing_from_above_to_below() {
		assert!(crossed_below_threshold(6, 4, 5));
	}

	#[test]
	fn no_emit_when_still_above() {
		assert!(!crossed_below_threshold(10, 6, 5));
	}

	#[test]
	fn no_emit_when_below_stays_below() {
		assert!(!crossed_below_threshold(4, 3, 5));
	}

	#[test]
	fn no_emit_when_equal_increasing() {
		assert!(!crossed_below_threshold(5, 6, 5));
	}
}