use crate::{roles::Role, SecurityContext, SecurityError};
use once_cell::sync::Lazy;
use prometheus::{IntCounterVec, Opts};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    InventoryView,
    CustomerView,
    CustomerWrite,
    PaymentProcess,
    LoyaltyView,
    GdprManage,
}

// Simple mapping: which roles are allowed each capability.
fn allowed_roles(cap: Capability) -> &'static [Role] {
    use Capability::*;
    use Role::*;
    match cap {
    // Added Cashier to InventoryView so POS/frontline roles can view stock levels (align tests & business need)
    InventoryView => &[SuperAdmin, Admin, Manager, Inventory, Cashier],
        CustomerView => &[SuperAdmin, Admin, Manager, Inventory, Cashier, Support],
        // Refined: CustomerWrite now excludes Inventory & Cashier to tighten scope (TA-POL-5)
        CustomerWrite => &[SuperAdmin, Admin, Manager],
        // Refined: PaymentProcess excludes Inventory role (Inventory no longer implicit payment access)
        PaymentProcess => &[SuperAdmin, Admin, Manager, Cashier],
        // LoyaltyView: support remains excluded pending future read-only loyalty capability
        LoyaltyView => &[SuperAdmin, Admin, Manager, Inventory, Cashier],
        // GdprManage: currently only high-privilege roles
        GdprManage => &[SuperAdmin, Admin],
    }
}

pub fn ensure_capability(ctx: &SecurityContext, cap: Capability) -> Result<(), SecurityError> {
    let allowed = allowed_roles(cap);
    if ctx.roles.iter().any(|r| allowed.iter().any(|a| a == r)) {
        CAPABILITY_CHECKS_TOTAL.with_label_values(&[cap.as_str(), "allow"]).inc();
        return Ok(());
    }
    tracing::warn!(?cap, roles = ?ctx.roles, tenant_id = %ctx.tenant_id, "capability_denied");
    CAPABILITY_DENIALS_TOTAL.with_label_values(&[cap.as_str()]).inc();
    CAPABILITY_CHECKS_TOTAL.with_label_values(&[cap.as_str(), "deny"]).inc();
    Err(SecurityError::Forbidden)
}

#[cfg(feature = "kafka")]
pub async fn emit_capability_denial_audit(
    producer: Option<&common_audit::BufferedAuditProducer<common_audit::KafkaAuditSink>>,
    ctx: &SecurityContext,
    cap: Capability,
    source_service: &str,
) {
    if let Some(p) = producer {
        let payload = serde_json::json!({
            "capability": format!("{:?}", cap),
            "roles": ctx.roles.iter().map(|r| format!("{:?}", r)).collect::<Vec<_>>()
        });
        let _ = p.emit(
            ctx.tenant_id,
            ctx.actor.clone(),
            "authorization",
            None,
            "capability_denied",
            source_service,
            common_audit::AuditSeverity::Security,
            ctx.trace_id,
            payload,
            serde_json::json!({})
        ).await; // ignore errors (non-critical path)
    }
}

// ---- Metrics (Capability Denials + Checks) ----
static CAPABILITY_DENIALS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("capability_denials_total", "Total count of capability authorization denials by capability"),
        &["capability"],
    ).expect("capability_denials_total");
    let _ = prometheus::default_registry().register(Box::new(c.clone()));
    c
});

static CAPABILITY_CHECKS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("capability_checks_total", "Total capability authorization checks by capability and outcome (allow|deny)"),
        &["capability", "outcome"],
    ).expect("capability_checks_total");
    let _ = prometheus::default_registry().register(Box::new(c.clone()));
    c
});

impl Capability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Capability::InventoryView => "inventory_view",
            Capability::CustomerView => "customer_view",
            Capability::CustomerWrite => "customer_write",
            Capability::PaymentProcess => "payment_process",
            Capability::LoyaltyView => "loyalty_view",
            Capability::GdprManage => "gdpr_manage",
        }
    }
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
        assert!(ensure_capability(&ctx, Capability::CustomerWrite).is_err(), "Cashier should not retain CustomerWrite after refinement");
    }

    #[test]
    fn superadmin_has_all() {
        let ctx = mk_ctx(vec![Role::SuperAdmin]);
        for cap in [Capability::InventoryView, Capability::CustomerView, Capability::CustomerWrite, Capability::PaymentProcess, Capability::LoyaltyView, Capability::GdprManage] { 
            assert!(ensure_capability(&ctx, cap).is_ok(), "SuperAdmin missing {:?}", cap);
        }
    }
}
