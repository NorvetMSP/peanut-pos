use crate::{roles::Role, SecurityContext, SecurityError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    InventoryView,
    CustomerView,
    CustomerWrite,
    PaymentProcess,
    LoyaltyView,
}

// Simple mapping: which roles are allowed each capability.
fn allowed_roles(cap: Capability) -> &'static [Role] {
    use Capability::*;
    use Role::*;
    match cap {
        InventoryView => &[SuperAdmin, Admin, Manager, Inventory, Cashier],
        CustomerView => &[SuperAdmin, Admin, Manager, Inventory, Cashier, Support],
        CustomerWrite => &[SuperAdmin, Admin, Manager, Inventory, Cashier],
        PaymentProcess => &[SuperAdmin, Admin, Manager, Inventory, Cashier],
        LoyaltyView => &[SuperAdmin, Admin, Manager, Inventory, Cashier],
    }
}

pub fn ensure_capability(ctx: &SecurityContext, cap: Capability) -> Result<(), SecurityError> {
    let allowed = allowed_roles(cap);
    if ctx.roles.iter().any(|r| allowed.iter().any(|a| a == r)) { return Ok(()); }
    Err(SecurityError::Forbidden)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use common_audit::AuditActor;

    fn mk_ctx(roles: Vec<Role>) -> SecurityContext {
        SecurityContext { tenant_id: Uuid::new_v4(), actor: AuditActor { id: Some(Uuid::new_v4()), name: None, email: None }, roles, trace_id: None }
    }

    #[test]
    fn support_cannot_write_customer() {
        let ctx = mk_ctx(vec![Role::Support]);
        assert!(ensure_capability(&ctx, Capability::CustomerWrite).is_err(), "Support should not have write capability");
    }

    #[test]
    fn cashier_can_process_payment() {
        let ctx = mk_ctx(vec![Role::Cashier]);
        assert!(ensure_capability(&ctx, Capability::PaymentProcess).is_ok());
    }

    #[test]
    fn superadmin_has_all() {
        let ctx = mk_ctx(vec![Role::SuperAdmin]);
        for cap in [Capability::InventoryView, Capability::CustomerView, Capability::CustomerWrite, Capability::PaymentProcess, Capability::LoyaltyView] { 
            assert!(ensure_capability(&ctx, cap).is_ok(), "SuperAdmin missing {:?}", cap);
        }
    }
}
