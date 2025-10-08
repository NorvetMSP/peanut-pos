# RoadMap Execution Checklist (Sequential)

Note: See Architecture overview for system context and principles: [docs/Architecture/Architecture_10_7_2025.md](./Architecture/Architecture_10_7_2025.md)

MVP Gap tracker (source-of-truth statuses): [docs/MVP_GAP_BUILD.md](./MVP_GAP_BUILD.md)
Runbook companion (Change Log & Roadmap): [docs/runbook.md](./runbook.md#change-log--roadmap)

Legend: [ ] Not Started · [~] In Progress · [x] Done

Each task: Goal (implicit in title), Actions, Acceptance, Dependencies.

## Program Overview — NovaPOS MVP (observability-first, tenant-aware)

What we’re building

- A tenant-aware, event-driven POS platform with clear observability, consistent APIs, and modular services (Orders, Payments, Inventory, Loyalty, Returns, Auth, Integration Gateway, Analytics). Admin Portal and POS Edge clients complete the user surface.

Guiding pillars

- Uniform endpoints and contracts (health, metrics, auth, idempotency)
- Event-first integrations (Kafka), transactional core where necessary (Postgres)
- Security-by-default (tenant context, RBAC, audit) and privacy considerations
- Observability and SLOs (standard `/metrics`, `/healthz`, stable labels, budgets)
- Iterative delivery with doc/code alignment and CI enforcement

Scope map (current status)

- Inventory: multi-location + basic reservations implemented; low-stock events emitted (UI pending)
- Returns/Exchanges: Exchange flow implemented; admin and POS E2E passed; policy engine pending
- Loyalty: Points read and earn-on-order completed; redemption pending
- Admin: Settlement report implemented; broader user management and global audit views pending
- Security/Audit: Shared security ctx and audit foundations in two services; consumer + retention + redaction in place per change log; remaining services pending
- Observability: `/metrics` standardized across services; generator produces service matrix; docs synced and enforced

---

## Phase 0 – Baseline & Observability

- [x] P0-01 Standardize metrics endpoints
  Actions: Add `/metrics` routes to all services; keep `/internal/metrics` aliases temporarily.
  Acceptance: Hitting `/metrics` on each service returns Prometheus exposition; CI/doc matrix reflects `/metrics`.
  Dependencies: None.

- [x] P0-02 Regenerate service matrix & enforce drift checks
  Actions: PowerShell generator reads compose/.env/topic map; regenerate matrix in Architecture doc. Wire pre-commit/CI to fail on drift.
  Acceptance: Matrix regenerated; no broken links; CI passing with drift gate.
  Dependencies: P0-01.

- [x] P0-03 Update runbook and quick links
  Actions: Update Architecture doc quick links, runbook curl checks to use `/metrics`; verify health/metrics paths across services.
  Acceptance: Links resolve; sample curls succeed locally.
  Dependencies: P0-01.

---

## Phase 1 – Inventory Multi-Location & Reservations

- [x] P1-01 Multi-location inventory schema & queries
  Actions: Add `locations`, augment `inventory_items(location_id)`, backfill, aggregate queries.
  Acceptance: Migrations 4003–4005 applied; queries return per-location and aggregated views; tests pass.
  Dependencies: None.

- [x] P1-02 Reservation lifecycle with expiration
  Actions: Create/release endpoints; sweeper job to expire; emit audit and domain events; restock on expire.
  Acceptance: Integration test validates restock + events; metrics emitted.
  Dependencies: P1-01.

- [~] P1-03 Low-stock alerts and UI surfacing
  Actions: Emit `inventory.low_stock` on threshold; add admin UI listing and threshold management.
  Acceptance: Events present; admin page shows low-stock list; thresholds configurable.
  Dependencies: P1-02.

---

## Phase 2 – Tenancy, RBAC, and Audit Foundations

- [~] P2-01 Shared tenancy + RBAC middleware rollout
  Actions: Migrate remaining services to common security crate; remove per-service duplicates.
  Acceptance: All mutating endpoints enforce role checks; duplication eliminated.
  Dependencies: None.

- [x] P2-02 Audit schema, sink, and producer in core services
  Actions: Common audit schema v1; sink abstraction; integrate in product & order services.
  Acceptance: Audit events emitted for CRUD and order lifecycle; tests/metrics present.
  Dependencies: None.

- [x] P2-03 Audit consumer, retention, and redaction
  Actions: Consumer service persists and exposes metrics; retention TTL job; redaction layer with modes.
  Acceptance: Consumer running; retention job metrics; redaction configurable; change log entries present.
  Dependencies: P2-02.

- [~] P2-04 Audit query API and admin views
  Actions: `/audit/events` read API (filters, pagination); admin portal search UI.
  Acceptance: API returns filtered, paginated results; basic admin UI browses events.
  Dependencies: P2-03.

---

## Phase 3 – Returns & Exchanges Foundations

- [x] P3-01 Exchange flow
  Actions: Implement exchange endpoint in order-service with delta/adjustments; E2E in POS/Admin.
  Acceptance: E2E happy path passes; inventory adjustments correct.
  Dependencies: P1-02.

- [ ] P3-02 Return policy module
  Actions: `return_policies` schema; apply to calculations and UI; audit events on changes.
  Acceptance: Policy applied in return calculation endpoint; UI displays fees/conditions; audit persisted.
  Dependencies: P3-01.

- [ ] P3-03 Manager override + audit
  Actions: Role-gated override endpoint; audit events on override.
  Acceptance: Overrides recorded and visible in audit views.
  Dependencies: P2-04, P3-02.

---

## Phase 4 – Loyalty Program

- [x] P4-01 Points read + earn accrual
  Actions: `/points` read; consume `order.completed` to earn; persist balances.
  Acceptance: Read and accrual paths tested; metrics present.
  Dependencies: P1-02.

- [ ] P4-02 Redemption / burn / expiry
  Actions: Transactional decrement; expiry model; safeguards.
  Acceptance: Redemption endpoints tested; consistency maintained under concurrency.
  Dependencies: P4-01.

- [ ] P4-03 Tiering & promotions
  Actions: Tier rules engine; promotions hooks.
  Acceptance: Tier assignments computed; hooks invoked on accrual/burn.
  Dependencies: P4-02.

---

## Phase 5 – Offline Orders & Replay Validation

- [ ] P5-01 Server-side replay validation
  Actions: Order-service validation endpoint referencing inventory and price; diff contract.
  Acceptance: Replay detects conflicts and returns deterministic diff response.
  Dependencies: P1-01.

- [ ] P5-02 Pending watchdog + reconciliation
  Actions: Background job for stalled PENDING orders; metrics and reconciliation.
  Acceptance: Watchdog metrics; stalled orders resolved or surfaced.
  Dependencies: P5-01.

- [ ] P5-03 Offline conflict resolution UI
  Actions: POS modal with diff, adjust/override/cancel options.
  Acceptance: E2E flow exercised; audit events for overrides.
  Dependencies: P5-01.

---

## Phase 6 – Payments & Gateway Foundations

- [ ] P6-01 Payment intent model
  Actions: `payment_intents` table; idempotent create; state transitions; reversal link.
  Acceptance: Idempotency enforced; transitions tested; persistence stable.
  Dependencies: P2-01.

- [ ] P6-02 Webhook hardening
  Actions: HMAC signature verification; nonce storage; replay detection.
  Acceptance: Valid signatures accepted; replays rejected; audit events recorded.
  Dependencies: P6-01.

- [ ] P6-03 Refund/reversal passthrough
  Actions: Gateway abstraction and endpoints for reversal; mapping from orders.
  Acceptance: Reversal flows complete; linked to original tender.
  Dependencies: P6-01, P6-02.

---

## Phase 7 – Admin & Management

- [~] P7-01 User management CRUD & roles
  Actions: Full lifecycle (create/edit/deactivate/role changes) with audit events.
  Acceptance: Admin UI actions persist; audit recorded; role enforcement consistent.
  Dependencies: P2-01, P2-02.

- [ ] P7-02 Inventory oversight UI
  Actions: Low-stock list; adjustments history; placeholder actions.
  Acceptance: Admin page shows stock/alerts; actions gated by role.
  Dependencies: P1-03.

- [ ] P7-03 Global audit views
  Actions: Search UI for `/audit/events` with filters and pagination.
  Acceptance: Browseable UI with performant queries.
  Dependencies: P2-04.

---

## Phase 8 – BOPIS / Fulfillment

- [~] P8-01 Promise/reservation layer
  Actions: Extend reservations with statuses/expirations and SLAs.
  Acceptance: Promises visible; expirations enforced; audit present.
  Dependencies: P1-02.

- [ ] P8-02 Pickup workflow states
  Actions: Domain model and APIs (ready, picked_up, expired, cancelled).
  Acceptance: State transitions recorded; inventory reconciled.
  Dependencies: P8-01.

---

## Phase 9 – Offline Sync Layer

- [ ] P9-01 Conflict diff service
  Actions: Endpoint compares submitted vs authoritative values; structured diff.
  Acceptance: Returns deterministic diff format; used by POS.
  Dependencies: P5-01.

- [ ] P9-02 Retry telemetry & dashboards
  Actions: Metrics endpoint for queue depth/failure counts; dashboards.
  Acceptance: Metrics scraped; dashboard panel green.
  Dependencies: P9-01.

- [ ] P9-03 Duplicate prevention beyond idempotency
  Actions: Hashing/content-based suppression.
  Acceptance: Duplicate rate reduced; metrics demonstrate effect.
  Dependencies: P9-01.

---

## Phase 10 – Cross-Cutting Architecture

- [ ] P10-01 API versioning standard
  Actions: Adopt `/v1` routing pattern; deprecation policy.
  Acceptance: Services expose versioned endpoints; docs updated.
  Dependencies: None.

- [ ] P10-02 Idempotency header contract
  Actions: Standardize header and persistence; align with orders and payments.
  Acceptance: Contract documented; code in place; tests green.
  Dependencies: P6-01.

- [ ] P10-03 Saga orchestration skeleton
  Actions: Introduce outbox or a lightweight orchestrator; start with returns & exchanges.
  Acceptance: Initial sagas defined; metrics present; failure semantics documented.
  Dependencies: P3-01, P6-01.

- [ ] P10-04 Timezone/reporting boundary
  Actions: Shared lib for timezones; reporting conversions.
  Acceptance: Consistent usage across services; docs and tests added.
  Dependencies: None.

---

## Notes

- This checklist mirrors the MVP_GAP_BUILD tracker, but sequences delivery to minimize churn and risk.
- Use this doc alongside the Architecture overview and MVP Gap tracker; keep statuses in sync (CI lint can enforce).
- When a task flips to [x], move any residual TODOs into the backlog section of MVP_GAP_BUILD.
