pub mod test_macros;
pub mod context;
pub mod error;
pub mod roles;
pub mod policy;

pub use context::{SecurityContext, SecurityCtxExtractor};
pub use error::SecurityError;
pub use roles::{ensure_role, ensure_any_role, Role};
pub use policy::{Capability, ensure_capability};
#[cfg(feature = "kafka")]
pub use policy::emit_capability_denial_audit;
