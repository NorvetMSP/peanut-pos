pub mod inventory_handlers;
pub mod reservation_handlers;
pub mod location_handlers;
mod main_impl_placeholder {} // placeholder to avoid pulling main
pub use crate::inventory_handlers::*;
pub use crate::reservation_handlers::*;
pub use crate::location_handlers::*;
pub const DEFAULT_THRESHOLD: i32 = 5;
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