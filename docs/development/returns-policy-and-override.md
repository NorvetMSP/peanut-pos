# Returns Policy and Manager Override (MVP)

Goal

- Apply configurable return policies during refunds/exchanges; allow role-gated manager overrides with audit.

Policy schema (order-service)

- Table: return_policies (tenant_id, location_id?, allow_window_days, restock_fee_bps, receipt_required, manager_override_allowed, updated_at)
- Precedence: location-specific > tenant-default
- Defaults: allow_window_days=30, restock_fee_bps=0, receipt_required=true, manager_override_allowed=true

API shape

- GET /admin/return_policies?location_id=... → effective policy row (requires Admin/Manager)
- POST /admin/return_policies { location_id?, allow_window_days, restock_fee_bps, receipt_required, manager_override_allowed }
  - Upsert for scope; requires Admin/Manager; emits audit event policy.updated
- POST /orders/:order_id/refund (existing) applies policy: window + fees
  - If outside window or fails receipt_required: 400 policy_violation unless override token present

Manager override

- POST /admin/overrides/returns { order_id, reason, code? } → returns override_token
  - Requires Manager role; persists override record with reason & actor; audit events override.issued
- Use: include header X-Return-Override: \<token\> on refund/exchange calls to bypass policy checks
  - Server validates token is recent (e.g., <= 15 minutes), scope matches tenant/order, not used before; audit override.used

Audit events

- policy.updated, override.issued, override.used, refund.policy_violation
- Attributes: tenant_id, user_id, order_id, location_id, reason, previous/new values

UI implications (Admin)

- Simple form to set policy per tenant/location
- List/table of overrides issued (last 7 days) with reasons

POS implications

- If refund blocked: show policy reason and “Request Manager Override” button
- Call override endpoint; on success, retry refund with header

Security & RBAC

- Admin/Manager to view/update policy; Manager to issue overrides; Cashier cannot

Follow-ups

- Add SKU/category-level exceptions (future)
- Anti-fraud thresholds & approval routing (ties to P3-06, P12)

Acceptance for MVP

- Policy row persists and is returned via GET; refund endpoint enforces window/receipt flags; override token bypass flow stubs exist and audit hooks are placed.
