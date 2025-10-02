# Capability & Authorization Reference (TA-DOC-3)

Status: Draft (Sprint A)

Owner: Platform / Security Engineering

Applies To: All HTTP services adopting `SecurityCtxExtractor` + `ensure_capability`

## Overview

Capabilities express business-permission intent independent of concrete role labels. Handlers enforce capability checks; roles are mapped centrally. This reduces coupling when role taxonomy evolves (e.g. adding RegionalManager) or when we trial an external policy engine.

## Current Capability Set (Refined TA-POL-5)

| Capability | Purpose | Typical Operations / Endpoints |
|------------|---------|---------------------------------|
| InventoryView | Read stock quantities & reservations | GET /inventory, list locations, reservation lookups |
| CustomerView | Read customer profile & search | GET /customers/:id, search customers |
| CustomerWrite | Create/update non-GDPR customer data | POST/PUT/PATCH /customers, profile edits |
| PaymentProcess | Initiate or void payments | POST /payments, POST /payments/:id/void |
| LoyaltyView | Retrieve loyalty balances or points history | GET /points, loyalty summaries |
| GdprManage | Execute GDPR-sensitive operations (erase/export) | DELETE /customers/:id (erase), export endpoints |

(Addition of new capabilities requires updating: policy mapping, deny tests, documentation, regression harness.)

## Role → Capability Mapping (After Refinement TA-POL-5)

| Role | InventoryView | CustomerView | CustomerWrite | PaymentProcess | LoyaltyView | GdprManage |
|------|---------------|--------------|---------------|----------------|------------|------------|
| SuperAdmin | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Admin | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Manager | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ |
| Inventory | ✓ | ✓ | ✗ | ✗ | ✓ | ✗ |
| Cashier | ✗ | ✓ | ✗ | ✓ | ✓ | ✗ |
| Support | ✗ | ✓ | ✗ | ✗ | ✗ | ✗ |

Legend: ✓ allowed, ✗ denied. Transitional allowances removed; matrix now principle-of-least-privilege aligned.

## Mapping Rationale

- SuperAdmin: Full override for operational recovery / break-glass.
- Admin: Full standard administrative coverage.
- Manager: Mirrors Admin for MVP simplicity; future differentiation may restrict CustomerWrite edge cases (PII redaction flows).
- Inventory: Limited strictly to inventory data (lost PaymentProcess and CustomerWrite).
- Cashier: Restricted to payment processing + viewing customers (write removed; POS write flow to use privileged path or future scoped capability).
- Support: Read-only customer view only.
- GdprManage: Constrained to Admin/SuperAdmin for sensitive erase/export operations.

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

Add deny + allow per capability per relevant service. Post-refinement focus tests:

- Cashier denied InventoryView (ensure 403 missing_role on inventory endpoints in gateway/inventory-service if attempted).
- Inventory role denied PaymentProcess.
- Cashier denied CustomerWrite.
- Non-admin roles denied GdprManage (customer-service GDPR handlers).


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
- 2025-10-02: Refinement (TA-POL-5) – tightened CustomerWrite & PaymentProcess, added GdprManage, removed transitional allowances.
