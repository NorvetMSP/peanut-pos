use payment_service::PAYMENT_ROLES;
use common_security::{roles::{ensure_any_role, Role}, SecurityCtxExtractor};
use common_security::context::SecurityContext;
use uuid::Uuid;
use common_audit::AuditActor;

fn mk_ctx(role: Role) -> SecurityContext {
    SecurityContext {
        tenant_id: Uuid::new_v4(),
        actor: AuditActor { id: Some(Uuid::new_v4()), name: None, email: None },
        roles: vec![role],
        trace_id: None,
    }
}

#[test]
fn cashier_role_allowed_for_payment() {
    let ctx = mk_ctx(Role::Cashier);
    ensure_any_role(&ctx, PAYMENT_ROLES).expect("cashier should be permitted in PAYMENT_ROLES");
}

#[test]
fn superadmin_role_allowed_for_payment() {
    let ctx = mk_ctx(Role::SuperAdmin);
    ensure_any_role(&ctx, PAYMENT_ROLES).expect("superadmin should be permitted in PAYMENT_ROLES");
}