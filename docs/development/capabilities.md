# Capability & Authorization Reference (TA-DOC-3)

Status: Draft (Sprint A)

Owner: Platform / Security Engineering

Applies To: All HTTP services adopting `SecurityCtxExtractor` + `ensure_capability`

## Overview

Capabilities express business-permission intent independent of concrete role labels. Handlers enforce capability checks; roles are mapped centrally. This reduces coupling when role taxonomy evolves (e.g. adding RegionalManager) or when we trial an external policy engine.

## Current Capability Set

| Capability | Purpose | Typical Operations / Endpoints |
|------------|---------|---------------------------------|
| InventoryView | Read stock quantities & reservations | GET /inventory, list locations, reservation lookups |
| CustomerView | Read customer profile & search | GET /customers/:id, search customers |
| CustomerWrite | Create/update customer data | POST/PUT/PATCH /customers, profile edits |
| PaymentProcess | Initiate or void payments | POST /payments, POST /payments/:id/void |
| LoyaltyView | Retrieve loyalty balances or points history | GET /points, loyalty summaries |

(Addition of new capabilities requires updating: policy mapping, deny tests, documentation, regression harness.)

## Role → Capability Mapping (Effective Today)

| Role | InventoryView | CustomerView | CustomerWrite | PaymentProcess | LoyaltyView |
|------|---------------|--------------|---------------|----------------|------------|
| SuperAdmin | ✓ | ✓ | ✓ | ✓ | ✓ |
| Admin | ✓ | ✓ | ✓ | ✓ | ✓ |
| Manager | ✓ | ✓ | ✓ | ✓ | ✓ |
| Inventory | ✓ | ✗ (future: limited?) | ✗ | ✓ (transitional) | ✓ |
| Cashier | ✓ | ✓ | ✓ (limited profile edits) | ✓ | ✓ |
| Support | ✗ | ✓ | ✗ | ✗ | ✗ |

Legend: ✓ allowed, ✗ denied. (Transitional notes reflect temporary allowances prior to matrix hardening.)

## Mapping Rationale

- SuperAdmin: Full override for operational recovery / break-glass.
- Admin: Full standard administrative coverage.
- Manager: Mirrors Admin for MVP simplicity; future differentiation may restrict CustomerWrite edge cases (PII redaction flows).
- Inventory: Focus on physical stock + payment bridging; intentionally excluded from CustomerWrite pending privacy review.
- Cashier: Needs customer create/update for point-of-sale onboarding; limited to surface-level attributes (enforced at field-level later).
- Support: Read-only access to customer profiles to resolve tickets; everything else denied to minimize lateral movement risk.

## Enforcement Pattern

```rust
use common_security::{ensure_capability, Capability};
use common_http_errors::ApiError;

async fn handler(SecurityCtxExtractor(sec): SecurityCtxExtractor) -> Result<Json<T>, ApiError> {
    ensure_capability(&sec, Capability::CustomerWrite)
        .map_err(|_| ApiError::ForbiddenMissingRole { role: "customer_write", trace_id: sec.trace_id })?;
    // proceed
}
```

## Deny/Allow Test Template

```rust
#[tokio::test]
async fn support_denied_customer_write() { /* assert 403 missing_role */ }
#[tokio::test]
async fn cashier_allowed_customer_write_path() { /* assert downstream 200/404 success */ }
```

Add one deny and one allow per capability per service where that capability is relevant. Inventory tests: support denied, cashier allowed already implemented.

## Adding a New Capability

1. Add variant to `Capability` enum (`common-security/src/policy.rs`).
2. Extend `allowed_roles` match with explicit list (prefer minimal principle-of-least-privilege default).
3. Add unit tests in `policy.rs` for at least: restricted role denial + one allowed role + SuperAdmin override.
4. Add handler enforcement (use `ensure_capability`). Remove any legacy role array branches.
5. Add deny/allow regression tests in the affected service.
6. Update this document’s tables.
7. Update backlog: new TA-POL-* item if major.

## Removing Legacy Role Fallbacks

Legacy `ensure_any_role` fallback branches have been removed (see addendum entry 2025-10-02). Any future handler must not reintroduce role-array logic; use capabilities exclusively.

## Future Evolution

- External Policy Engine (TA-POL-2): Capabilities become resource/action pairs (e.g. `customer:write`) for dynamic evaluation.
- Field-Level Constraints: `CustomerWrite` subdivides into `CustomerPIIWrite` vs `CustomerBasicWrite` once encryption / masking policies mature.
- Auditing: Capability decision successes/failures exported as structured audit events for forensic review.

## Open Questions

- Should Inventory role retain PaymentProcess long-term? (Risk: over-permission)
- Introduce a distinct Reporting capability for analytics endpoints?
- Support read-only scoping enforcement at database layer vs handler?

## Quick Reference Cheat Sheet

| Error Scenario | Code | HTTP Status | Header `X-Error-Code` |
|----------------|------|-------------|-----------------------|
| Missing tenant header | missing_tenant_id | 400 | missing_tenant_id |
| Capability denied | missing_role | 403 | missing_role |
| Resource not found | `your_resource`_not_found | 404 | `your_resource`_not_found |
| Internal error | internal_error | 500 | internal_error |

## Changelog

- 2025-10-02: Initial draft (Sprint A) – baseline capabilities + deny matrix rationale.
