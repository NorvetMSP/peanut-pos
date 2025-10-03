use common_security::roles::Role;

// Re-export role arrays for integration tests and other binaries.
pub const CUSTOMER_WRITE_ROLES: &[Role] = &[Role::SuperAdmin, Role::Admin, Role::Manager, Role::Inventory, Role::Cashier];
pub const CUSTOMER_VIEW_ROLES: &[Role]  = &[Role::SuperAdmin, Role::Admin, Role::Manager, Role::Inventory, Role::Cashier];

pub use common_security::SecurityCtxExtractor;