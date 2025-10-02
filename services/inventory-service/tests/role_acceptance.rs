use inventory_service::inventory_handlers::INVENTORY_VIEW_ROLES;
use common_security::roles::{ensure_any_role, Role};
use common_security::context::SecurityContext;
use uuid::Uuid;
use common_audit::AuditActor;

fn mk_ctx(role: Role) -> SecurityContext {
    SecurityContext { tenant_id: Uuid::new_v4(), actor: AuditActor { id: Some(Uuid::new_v4()), name: None, email: None }, roles: vec![role], trace_id: None }
}

#[test]
fn cashier_role_allowed_for_inventory_view() {
    let ctx = mk_ctx(Role::Cashier);
    ensure_any_role(&ctx, INVENTORY_VIEW_ROLES).expect("cashier should be allowed for inventory view");
}

#[test]
fn superadmin_role_allowed_for_inventory_view() {
    let ctx = mk_ctx(Role::SuperAdmin);
    ensure_any_role(&ctx, INVENTORY_VIEW_ROLES).expect("superadmin should be allowed for inventory view");
}