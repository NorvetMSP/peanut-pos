pub mod alerts;
pub mod config;
pub mod events;
pub mod integration_handlers;
pub mod metrics;
pub mod rate_limiter;
pub mod usage;
pub mod webhook_handlers;
pub mod app_state;

// Re-export key types for tests
pub use crate::integration_handlers::{void_payment, PaymentVoidRequest, PaymentResult};
pub use crate::app_state::{AppState, CachedKey};
pub use crate::metrics::GatewayMetrics;
pub use crate::rate_limiter::RateLimiter;
pub use crate::usage::UsageTracker;
pub use crate::config::GatewayConfig;
pub use uuid::Uuid;

// AppState lives in main.rs; replicate minimal struct subset for tests that only need handler invocation without full server.
// For now we skip re-exporting AppState because the void_payment handler only requires State(AppState) but our integration test
// directly routes handler without providing AppState (it marks the state argument unused). If future tests need full state, we can
// refactor AppState into a shared module and re-export it here.
