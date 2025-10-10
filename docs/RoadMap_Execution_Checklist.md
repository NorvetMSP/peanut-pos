# RoadMap Execution Checklist (Sequential)

Note: See Architecture overview for system context and principles: [docs/Architecture/Architecture_10_7_2025.md](./Architecture/Architecture_10_7_2025.md)

MVP Gap tracker (source-of-truth): [docs/MVP_Gaps.md]
MVP Gap tracker (source-of-truth statuses): [docs/MVP_GAP_BUILD.md](./MVP_GAP_BUILD.md)
Runbook companion (Change Log & Roadmap): [docs/runbook.md](./runbook.md#change-log--roadmap)

Legend: [ ] Not Started · [~] In Progress · [x] Done

Each task: Goal (implicit in title), Actions, Acceptance, Dependencies.

Quick index: [Cashier MVP Addendum](#addendum--cashier-mvp-critical-path--proposed-additions)

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

## Phase 0.1 – KPI & Messaging Observability

- [~] P0-04 End-to-end KPI instrumentation [cashier-mvp]
  Actions: Add distributed tracing across POS → Order → Inventory → Payment → Kafka; expose checkout latency histograms and tap-count metrics with stable labels by tenant/store.
  Acceptance: Dashboards display p50/p95 checkout latency and tap counts per tenant/store; alerting wired to SLOs (<2s, <5 taps) with test alerts.
  Dependencies: P0-01.

- [ ] P0-05 Kafka lag and topic health
  Actions: Export consumer lag for primary topics (`order.completed`, `payment.completed`, `order.voided`); add Grafana panels and alerts.
  Acceptance: Lag panels present and green in steady state; alert triggers on configured thresholds; runbook linked.
  Dependencies: P0-02.

- [x] P0-06 POS offline queue telemetry [cashier-mvp]
  Actions: Emit offline queue depth/failure metrics from POS; backend aggregates per-tenant; dashboards and alerts.
  Acceptance: Dashboard shows offline queue depth by store; alert on prolonged backlog; synthetic tests validate ingestion.
  Dependencies: P9-02.
  Notes: POS emits local counters/gauges for print retries and queue depth (see `pos-receipts.md`). A scheduler batches and POSTs snapshots when `VITE_TELEMETRY_INGEST_URL` is set, labeled by `tenant_id`/`store_id`. Backend ingestion endpoint in `order-service` maps POS payloads to Prometheus metrics (`pos_print_retry_total`, `pos_print_gauge`); Grafana dashboard visualizes queue depth and retry counters; Prometheus alert rules are wired and validated locally. Runbook includes a smoke test and a PowerShell helper script (`scripts/post-pos-telemetry.ps1`).

---

## Phase 1 – Inventory Multi-Location & Reservations

- [x] P1-01 Multi-location inventory schema & queries [cashier-mvp]
  Actions: Add `locations`, augment `inventory_items(location_id)`, backfill, aggregate queries.
  Acceptance: Migrations 4003–4005 applied; queries return per-location and aggregated views; tests pass.
  Dependencies: None.

- [x] P1-02 Reservation lifecycle with expiration [cashier-mvp]
  Actions: Create/release endpoints; sweeper job to expire; emit audit and domain events; restock on expire.
  Acceptance: Integration test validates restock + events; metrics emitted.
  Dependencies: P1-01.

- [~] P1-03 Low-stock alerts and UI surfacing [cashier-mvp]
  Actions: Emit `inventory.low_stock` on threshold; add admin UI listing and threshold management.
  Acceptance: Events present; admin page shows low-stock list; thresholds configurable.
  Dependencies: P1-02.

---

## Phase 2 – Tenancy, RBAC, and Audit Foundations

- [~] P2-01 Shared tenancy + RBAC middleware rollout [cashier-mvp]
  Actions: Migrate remaining services to common security crate; remove per-service duplicates.
  Acceptance: All mutating endpoints enforce role checks; duplication eliminated.
  Dependencies: None.

- [x] P2-02 Audit schema, sink, and producer in core services [cashier-mvp]
  Actions: Common audit schema v1; sink abstraction; integrate in product & order services.
  Acceptance: Audit events emitted for CRUD and order lifecycle; tests/metrics present.
  Dependencies: None.

- [x] P2-03 Audit consumer, retention, and redaction [cashier-mvp]
  Actions: Consumer service persists and exposes metrics; retention TTL job; redaction layer with modes.
  Acceptance: Consumer running; retention job metrics; redaction configurable; change log entries present.
  Dependencies: P2-02.

- [~] P2-04 Audit query API and admin views [cashier-mvp]
  Actions: `/audit/events` read API (filters, pagination); admin portal search UI.
  Acceptance: API returns filtered, paginated results; basic admin UI browses events.
  Dependencies: P2-03.

---

## Phase 2.1 – Security Hardening & Data Privacy

- [ ] P2-05 mTLS between services
  Actions: Enable mutual TLS for inter-service REST/Kafka where applicable (service mesh or app-level); implement cert rotation automation.
  Acceptance: All intra-service calls use mTLS in non-dev profiles; rotation procedure validated in lower env; metrics for cert expiry exposed.
  Dependencies: P0-01.

- [ ] P2-06 Per-tenant key management across services
  Actions: Adopt envelope encryption via Vault/KMS for services beyond Customer/Auth; unify key retrieval and caching patterns.
  Acceptance: Keys managed per-tenant in at least three services; rotation-ready with minimal downtime; documentation updated.
  Dependencies: P2-02.

- [ ] P2-07 Automated key rotation and DSR orchestration
  Actions: Implement rotation jobs; create a DSR coordinator API to orchestrate export/delete across services with auditing.
  Acceptance: Rotation dry-run and execution tested; DSR requests produce consistent multi-service results with audit trail.
  Dependencies: P2-06.

- [ ] P2-08 Immutable audit store
  Actions: Introduce append-only audit sink with tamper-evident hashing and retention; expose verification endpoint.
  Acceptance: Audit writes are immutable; verification checks pass; retention metrics visible.
  Dependencies: P2-03.

---

## Phase 3 – Returns & Exchanges Foundations

- [x] P3-01 Exchange flow [cashier-mvp]
  Actions: Implement exchange endpoint in order-service with delta/adjustments; E2E in POS/Admin.
  Acceptance: E2E happy path passes; inventory adjustments correct.
  Dependencies: P1-02.

- [~] P3-02 Return policy module [cashier-mvp]
  Actions: `return_policies` schema; apply to calculations and UI; audit events on changes.
  Acceptance: Policy applied in return calculation endpoint; UI displays fees/conditions; audit persisted.
  Dependencies: P3-01.

- [~] P3-03 Manager override + audit [cashier-mvp]
  Actions: Role-gated override endpoint; audit events on override.
  Acceptance: Overrides recorded and visible in audit views.
  Dependencies: P2-04, P3-02.

---

## Phase 3.1 – Returns Policy & Fraud Controls

- [ ] P3-04 Restock rules & compensation
  Actions: Configurable restock policies (location-aware) + automatic inventory adjustments; audit events.
  Acceptance: Returns adjust stock per policy; reports reconcile; audit entries emitted.
  Dependencies: P1-02, P3-02.

- [ ] P3-05 Link returns to original order lines
  Actions: Schema/API linking returns to original lines (and serials where applicable); enforce eligibility.
  Acceptance: API enforces linkage; E2E tests validate restock and accounting; analytics reflects linkage.
  Dependencies: P3-02.

- [ ] P3-06 Fraud flags & thresholds
  Actions: Rules for thresholds (frequency/amount), flags on return requests, manager approval gates.
  Acceptance: Threshold breaches require approval path; flags visible in admin; audit present.
  Dependencies: P3-03.

---

## Phase 4 – Loyalty Program

- [x] P4-01 Points read + earn accrual [cashier-mvp]
  Actions: `/points` read; consume `order.completed` to earn; persist balances.
  Acceptance: Read and accrual paths tested; metrics present.
  Dependencies: P1-02.

- [ ] P4-02 Redemption / burn / expiry [cashier-mvp]
  Actions: Transactional decrement; expiry model; safeguards.
  Acceptance: Redemption endpoints tested; consistency maintained under concurrency.
  Dependencies: P4-01.

- [ ] P4-03 Tiering & promotions
  Actions: Tier rules engine; promotions hooks.
  Acceptance: Tier assignments computed; hooks invoked on accrual/burn.
  Dependencies: P4-02.

---

## Phase 4.1 – Loyalty Offline & 360

- [ ] P4-04 Offline cache & reconciliation [cashier-mvp]
  Actions: POS offline balance cache with reconciliation job to correct missed accruals; idempotent adjustments.
  Acceptance: Offline accrual reconciles without double-counting; metrics for corrections exposed.
  Dependencies: P4-01, P5-01.

- [ ] P4-05 Accrual reversal & adjustments
  Actions: Reverse points on void/refund; admin adjustments with audit and role checks.
  Acceptance: Reversals consistent under concurrency; tests cover edge cases; audit logged.
  Dependencies: P3-01, P3-02.

- [ ] P4-06 Customer 360 aggregation API
  Actions: Aggregated profile endpoint (points, recent orders, preferences) for POS/Admin.
  Acceptance: Endpoint returns tenant-scoped 360 with p95 latency budget; basic UI surfacing.
  Dependencies: P4-01.

---

## Phase 5 – Offline Orders & Replay Validation

- [ ] P5-01 Server-side replay validation [cashier-mvp]
  Actions: Order-service validation endpoint referencing inventory and price; diff contract.
  Acceptance: Replay detects conflicts and returns deterministic diff response.
  Dependencies: P1-01.

- [ ] P5-02 Pending watchdog + reconciliation [cashier-mvp]
  Actions: Background job for stalled PENDING orders; metrics and reconciliation.
  Acceptance: Watchdog metrics; stalled orders resolved or surfaced.
  Dependencies: P5-01.

- [ ] P5-03 Offline conflict resolution UI [cashier-mvp]
  Actions: POS modal with diff, adjust/override/cancel options.
  Acceptance: E2E flow exercised; audit events for overrides.
  Dependencies: P5-01.

---

## Phase 5.1 – Order Integrity & Idempotency

- [ ] P5-04 Idempotency storage + outbox/inbox
  Actions: Persist idempotency keys; implement outbox pattern for event emission (Order/Payment/Inventory) and inbox de-duplication for consumers.
  Acceptance: Duplicate submissions do not create duplicate orders; events delivered exactly-once to business logic; chaos test demonstrates durability.
  Dependencies: P10-03.

- [ ] P5-05 Price-lock snapshots on order lines [cashier-mvp]
  Actions: Capture unit price/tax rule snapshot per line at submission; validate on replay.
  Acceptance: Replay diff can reconcile catalog changes deterministically; reporting consistent.
  Dependencies: P5-01.

---

## Phase 6 – Payments & Gateway Foundations

 - [x] P6-01 Payment intent model [cashier-mvp]
  Actions: `payment_intents` table; idempotent create; state transitions; reversal link.
  Acceptance: Endpoints for create/get/confirm/capture/void/refund operational; with DB configured, invalid transitions return 409 (`invalid_state_transition`); without DB, endpoints stub nominal states for local workflows. Order-service can optionally initiate intent on card checkout when `ENABLE_PAYMENT_INTENTS=1`.
  Notes: MVP implemented in `payment-service` with optional DB (falls back to stateless stubs if `DATABASE_URL` is unset). SQLx migration `8002_create_payment_intents.sql`; repository implements simple state transitions. New routes: `POST /payment_intents`, `GET /payment_intents/:id`, `POST /payment_intents/confirm` (capture/void/refund wired). Tests cover create→confirm happy path without DB. Next: enforce transitions, wire provider refs, and add DB-backed tests.
  Dependencies: P2-01.

- [~] P6-02 Webhook hardening [cashier-mvp]
  Actions: HMAC signature verification; nonce storage; replay detection.
  Acceptance: Valid signatures accepted; replays rejected; audit events recorded.
  Dependencies: P6-01.

- [~] P6-03 Refund/reversal passthrough [cashier-mvp]
  Actions: Gateway abstraction and endpoints for reversal; mapping from orders.
  Acceptance: Reversal flows complete; linked to original tender.
  Dependencies: P6-01, P6-02.

---

## Phase 6.1 – Tender Orchestration & Reconciliation

- [ ] P6-04 Split tender support [cashier-mvp]
  Actions: Payment intent sub-allocations with validation; POS/Admin updates; reconciliation and receipts.
  Acceptance: Orders complete with ≥2 tenders; totals reconcile; tests and receipts verified.
  Dependencies: P6-01.

- [ ] P6-05 Auth/capture/void lifecycle [cashier-mvp]
  Actions: Payment state machine supporting auth→capture→refund/void; timers for auto-void; metrics.
  Acceptance: State transitions enforced; auto-void works; observability present.
  Dependencies: P6-01.

- [ ] P6-06 Gateway references & nightly reconciliation
  Actions: Persist provider transaction refs; implement reconciliation job and drift reports.
  Acceptance: Drift report shows zero on happy path; alerts on mismatch; runbook documented.
  Dependencies: P6-01, P0-05.

---

## Phase 7 – Admin & Management

- [~] P7-01 User management CRUD & roles [cashier-mvp]
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

- [~] P8-01 Promise/reservation layer [cashier-mvp]
  Actions: Extend reservations with statuses/expirations and SLAs.
  Acceptance: Promises visible; expirations enforced; audit present.
  Dependencies: P1-02.

- [ ] P8-02 Pickup workflow states [cashier-mvp]
  Actions: Domain model and APIs (ready, picked_up, expired, cancelled).
  Acceptance: State transitions recorded; inventory reconciled.
  Dependencies: P8-01.

---

## Phase 8.1 – SLA & No‑show Management

- [ ] P8-03 SLA dashboards & no-show auto-release
  Actions: Timers and alerts for pickup SLAs; automatic release of expired reservations; dashboards.
  Acceptance: Expired promises auto-released; SLA panels green under normal ops; audit present.
  Dependencies: P8-01.

---

## Phase 9 – Offline Sync Layer

- [ ] P9-01 Conflict diff service [cashier-mvp]
  Actions: Endpoint compares submitted vs authoritative values; structured diff.
  Acceptance: Returns deterministic diff format; used by POS.
  Dependencies: P5-01.

- [x] P9-02 Retry telemetry & dashboards [cashier-mvp]
  Actions: Metrics endpoint for queue depth/failure counts; dashboards.
  Acceptance: Metrics scraped; dashboard panel green; alerts loaded and firing conditions verified in Prometheus UI.
  Dependencies: P9-01.
  Notes: Aggregation implemented via `order-service` ingestion of POS snapshots; Prometheus metrics exported with stable names/labels (e.g., `pos_print_retry_total` with kind labels; `pos_print_gauge` for queue depth). Grafana dashboard added for queue depth and retry trends; alert thresholds/rules defined and loaded in Prometheus; Alertmanager routing configured (env-specific receivers). Dependency with P9-01 remains for broader offline sync, but this telemetry is complete and independent.

- [ ] P9-03 Duplicate prevention beyond idempotency [cashier-mvp]
  Actions: Hashing/content-based suppression.
  Acceptance: Duplicate rate reduced; metrics demonstrate effect.
  Dependencies: P9-01.

---

## Phase 10 – Cross-Cutting Architecture

## Phase 11 – Catalog Variants & Serials

- [ ] P11-01 Variant schema and CRUD
  Actions: Add variants (e.g., size/color) to product domain; CRUD + validations.
  Acceptance: Variant creation/edit flows tested; API and Admin UI updated.
  Dependencies: None.

- [ ] P11-02 Per-variant inventory and barcode mapping [cashier-mvp]
  Actions: Track inventory at variant level; map barcodes to variants.
  Acceptance: Sales decrement correct variant stock; barcode scans resolve to variant.
  Dependencies: P11-01, P1-01.

- [ ] P11-03 Serial capture on sale/return
  Actions: Serial number capture and validation for serialized goods.
  Acceptance: POS enforces serial capture; returns validate original serial.
  Dependencies: P11-02.

- [ ] P11-04 Migration and backfill strategy
  Actions: Safe migration plan and backfill scripts for existing products.
  Acceptance: Migration executed in dev/staging; data integrity verified.
  Dependencies: P11-01.

## Phase 12 – Approvals & Mobility

- [ ] P12-01 Approval service and mobile tokens
  Actions: Central approval service; mobile approval tokens/links; integration with Admin/POS.
  Acceptance: Remote approvals complete critical flows; audit end-to-end.
  Dependencies: P2-01, P2-02.

- [ ] P12-02 Policy DSL and routing
  Actions: Define policy rules and routing for approvals by risk/amount.
  Acceptance: Policies configurable per tenant; tests for routing outcomes.
  Dependencies: P12-01.

- [ ] P12-03 Transaction audit linkage
  Actions: Link approvals immutably to orders/returns/payments.
  Acceptance: Audit view shows complete chain; integrity checks pass.
  Dependencies: P2-08.

## Phase 13 – Device & Peripherals Layer

- [~] P13-01 Device abstraction SDK [cashier-mvp]
  Actions: Unified interfaces for printer, scanner, payment terminal with fallbacks.
  Acceptance: POS uses SDK; mocks available for CI; errors surfaced gracefully.
  Dependencies: None.
  Notes: Print-only receipts wired for MVP via Device SDK printer interface; e‑receipt templates deferred (see P16-01). Print failure toast with Retry implemented; success toast confirms print. Branding resolved via tenant-config when available with env fallback (see docs/development/pos-receipts.md). Proactive printer status banner surfaces disconnected/error states. Snapshot tests added for receipt formatter.

- [~] P13-02 Hot-plug detection and retries [cashier-mvp]
  Actions: Detect device (dis)connect; retry queues/backoff; telemetry.
  Acceptance: Hot-plug scenarios pass; retries visible in metrics.
  Dependencies: P13-01.
  Notes: Event-driven device status added to the POS Device SDK; printer hot-plug detection surfaces status changes in the UI. Print jobs queue when the device is unavailable and auto-retry on reconnect with backoff. A "queued for retry" toast and proactive status banner are in place. Unit tests cover device status events and retry queue success paths. POS telemetry counters/gauges for print retries and queue depth emit locally and via a scheduler to a backend ingestion endpoint in `order-service`, which exposes Prometheus metrics consumed by a Grafana dashboard. Remaining: validate service build/deploy, wire alerting, and add synthetic tests.

- [ ] P13-03 Device telemetry and health [cashier-mvp]
  Actions: Emit device health metrics and logs; dashboards.
  Acceptance: Device panels show status per store; alerts on failures.
  Dependencies: P0-04.

- [ ] P13-04 Mock drivers for CI
  Actions: Provide mock drivers for automated tests.
  Acceptance: CI coverage includes device flows without hardware.
  Dependencies: P13-01.

## Phase 14 – Promotions & Price Governance

- [ ] P14-01 Promo engine and scheduling [cashier-mvp]
  Actions: Rules engine for promos/markdowns; scheduling and scopes.
  Acceptance: Promos applied deterministically; schedule honored; audit recorded.
  Dependencies: P10-01.

- [ ] P14-02 Approval workflows and simulation
  Actions: Approval gates for high-impact promos; simulation tooling to predict impact.
  Acceptance: Simulations match applied results; approvals audited.
  Dependencies: P12-01.

- [ ] P14-03 Backend enforcement across services
  Actions: Ensure promo enforcement in pricing/calculation paths.
  Acceptance: E2E checks show consistent pricing; tests cover edge cases.
  Dependencies: P14-01.

## Phase 15 – Data & Analytics Plumbing

- [ ] P15-01 Event contracts & schema registry
  Actions: Define versioned Kafka schemas and validation in CI.
  Acceptance: Breaking changes blocked by CI; consumers validated.
  Dependencies: P0-02.

- [ ] P15-02 CDC/ETL to warehouse
  Actions: Ship operational data to analytical store (e.g., BigQuery/Snowflake/Postgres replica) with transforms.
  Acceptance: Core tables populated; freshness/latency SLOs met; lineage documented.
  Dependencies: P15-01.

- [ ] P15-03 Anomaly detection baseline
  Actions: Implement basic anomaly detection over sales/returns/lag; alerting.
  Acceptance: Anomalies detected reliably in seeded scenarios; false positive rate acceptable.
  Dependencies: P15-02.

## Phase 16 – E‑receipts & Communications

- [ ] P16-01 Notifications service and templates [cashier-mvp]
  Actions: Multi-channel notifications (email/SMS/webhook) with templating and tenant branding.
  Acceptance: E‑receipt sent on order completion; retries and provider failover verified.
  Dependencies: P2-01.

- [ ] P16-02 Locale and branding
  Actions: Template localization and per-tenant branding assets.
  Acceptance: E‑receipts reflect locale and branding; snapshot tests.
  Dependencies: P16-01.

- [ ] P16-03 Delivery receipts and retries
  Actions: Store provider delivery receipts; retry policies; dead-letter handling.
  Acceptance: Delivery status tracked; retries visible in metrics; DLQ monitored.
  Dependencies: P16-01, P0-05.

## Phase 17 – Tasking & Store Checklists

- [ ] P17-01 Task service and API [cashier-mvp]
  Actions: CRUD for tasks/checklists; assignment to stores/users; schedules.
  Acceptance: Tasks created/assigned/completed with audit; APIs tested.
  Dependencies: P2-01.

- [ ] P17-02 POS/Admin integration [cashier-mvp]
  Actions: Surfaces open/close procedures and directives in POS/Admin.
  Acceptance: Staff can complete tasks; manager views show progress.
  Dependencies: P17-01.

## Phase 18 – Identity at Scale

- [ ] P18-01 SSO (OIDC/SAML) integration
  Actions: Integrate with external IdPs; configure per-tenant SSO.
  Acceptance: Users authenticate via IdP; fallback local admin account maintained.
  Dependencies: P2-01.

- [ ] P18-02 SCIM provisioning
  Actions: Automate user/group lifecycle via SCIM.
  Acceptance: Creates/updates/deactivations sync correctly; audit present.
  Dependencies: P18-01.

- [ ] P18-03 ABAC and scoped access controls
  Actions: Attribute-based access control and store scoping in RBAC layer.
  Acceptance: Policies enforced across services; tests cover attribute conditions.
  Dependencies: P2-01.

---

## Notes

This checklist mirrors the MVP_GAP_BUILD tracker, but sequences delivery to minimize churn and risk.

Use this doc alongside the Architecture overview and MVP Gap tracker; keep statuses in sync (CI lint can enforce).

When a task flips to [x], move any residual TODOs into the backlog section of MVP_GAP_BUILD.

---

## Appendix — Implementation pointers for new items

- P0-04 End-to-end KPI instrumentation
  - Services: `frontends/pos-app`, `services/order-service`, `services/payment-service`, `services/inventory-service`, `services/integration-gateway`
  - Files: add tracing in `src/main.rs` and HTTP handlers (`*_handlers.rs`); POS capture in `frontends/pos-app/src/OrderContext.tsx`
  - Notes: OpenTelemetry tracing, histogram buckets for latency; label by `tenant_id`, `store_id`

- P0-05 Kafka lag and topic health
  - Services: all Kafka consumers (`inventory-service`, `loyalty-service`, `analytics-service`, `order-service`)
  - Files: consumer init in `src/main.rs`; export lag via exporter or PromQL using Kafka exporter
  - Notes: Topics: `order.completed`, `payment.completed`, `order.voided`

- P0-06 POS offline queue telemetry
  - Services: `frontends/pos-app`
  - Files: `frontends/pos-app/src/OrderContext.tsx` — add gauge/counter emit; endpoint in `order-service` to ingest metrics if needed

- P2-05 mTLS between services
  - Services: all REST services; Kafka clients
  - Files: service TLS config in `src/main.rs`; compose/k8s manifests under `docker-compose.yml`, `infra/terraform/*`

- P2-06 Per-tenant key management across services
  - Services: `customer-service` (reference), extend to `order-service`, `inventory-service`
  - Files: key retrieval modules; env/Vault wiring in `src/main.rs`

- P2-07 Key rotation and DSR orchestration
  - Services: new coordinator under `services/` or extend `auth-service`
  - Files: rotation job module; `/dsr/*` API handlers

- P2-08 Immutable audit store
  - Services: dedicated audit sink or extend `analytics-service`
  - Files: append-only writer; hash chain; verification endpoint `/audit/verify`

- P3-04 Restock rules & compensation
  - Services: `inventory-service`
  - Files: policy evaluation in `src/inventory_handlers.rs` and domain modules; migration under `services/inventory-service/migrations`

- P3-05 Link returns to original order lines
  - Services: `order-service`
  - Files: schema migrations in `services/order-service/migrations`; handlers in `src/order_handlers.rs`

- P3-06 Fraud flags & thresholds
  - Services: `order-service`, `admin-portal`
  - Files: flags in return models; admin UI form additions under `frontends/admin-portal/src/pages/*`

- P4-04 Offline cache & reconciliation
  - Services: `frontends/pos-app`, `loyalty-service`
  - Files: POS cache in `OrderContext.tsx`; reconciliation job in `services/loyalty-service/src/main.rs`

- P4-05 Accrual reversal & adjustments
  - Services: `loyalty-service`, `order-service`
  - Files: consume `order.voided` in loyalty; admin adjustments endpoint in loyalty handlers

- P4-06 Customer 360 aggregation API
  - Services: new aggregator route in `customer-service` or `analytics-service`
  - Files: `/customer/360` handler; joins against orders/loyalty (read models)

- P5-04 Idempotency storage + outbox/inbox
  - Services: `order-service`, `payment-service`, `inventory-service`
  - Files: add `idempotency_keys` and `outbox` tables; emit via outbox worker; consumer-side inbox de-dup

- P5-05 Price-lock snapshots on order lines
  - Services: `order-service`
  - Files: extend order lines schema; write snapshot at submission; replay validation in `src/order_handlers.rs`

- P6-04 Split tender support
  - Services: `order-service`, `payment-service`, `frontends/pos-app`
  - Files: model multiple tenders in `payment_intents`; POS UI to allocate amounts; receipt updates

- P6-05 Auth/capture/void lifecycle
  - Services: `payment-service`
  - Files: state machine module; timers/auto-void worker; metrics

- P6-06 Gateway references & nightly reconciliation
  - Services: `payment-service`, `integration-gateway`
  - Files: persist provider refs; recon job; drift report route

- P8-03 SLA dashboards & no-show auto-release
  - Services: `inventory-service`
  - Files: scheduler for auto-release; metrics; admin visibility in `admin-portal`

- Phase 11 Variants & Serials
  - Services: `product-service`, `inventory-service`
  - Files: migrations for variants; per-variant inventory; serial capture on sale/return

- Phase 12 Approvals & Mobility
  - Services: new `approval-service` or extend `auth-service`; hooks in orders/returns
  - Files: approval endpoints; mobile token flows; audit linkage

- Phase 13 Device & Peripherals
  - Services: `frontends/pos-app`
  - Files: device SDK under `frontends/pos-app/src/devices/*`; mocks in tests

- Phase 14 Promotions
  - Services: `product-service` (pricing), `order-service` (calculation)
  - Files: promo rules engine modules; admin screens for promos

- Phase 15 Data & Analytics Plumbing
  - Services: schema registry in CI; ETL scripts under `monitoring/` or `scripts/`; consumer validation in services

- Phase 16 E‑receipts & Communications
  - Services: new `notifications-service` or extend `integration-gateway`
  - Files: templates store; providers; retries/DLQ

- Phase 17 Tasking & Checklists
  - Services: new `task-service`; UI in `admin-portal` and POS

- Phase 18 Identity at Scale
  - Services: `auth-service`
  - Files: OIDC/SAML config; SCIM endpoints; ABAC policies in common security crate

---

## Addendum — Cashier MVP: Critical Path & Proposed Additions

scope: cashier-mvp

This addendum aggregates the cashier journey items into a single reference without altering existing phases. It maps “already in place”, “in progress”, known gaps, and a recommended sequence for MVP, referencing IDs from this checklist.

### Already in Place (references)

- Inventory basics: multi-location and reservations — P1-01, P1-02 (scanning/building carts with stock control)
- Exchange flow (baseline exceptions) — P3-01
- Loyalty accrual on order complete (read + earn) — P4-01
- Audit foundations (emit + consume/retention/redaction) — P2-02, P2-03
- Program overview mirrors the above status — see Program Overview section

### In Progress (impacts cashier)

- RBAC middleware rollout — P2-01
- Audit query API + admin views — P2-04
- User management CRUD & roles — P7-01
- Low stock alerts (UI surfacing) — P1-03
- BOPIS promise/reservation layer — P8-01

### Gaps to Enable End-to-End Cashier

- Opening & setup
  - Device/peripherals: abstraction + hot plug + telemetry — P13-01, P13-02, P13-03
  - Offline login/grace mode for POS — Proposed (see below)
- Active sales & checkout
  - Payments foundation (intents, webhook hardening, refunds, auth/capture/void) — P6-01, P6-02, P6-03, P6-05
  - Split tender — P6-04
  - Printed/e‑receipts — P16-01 (or print-only receipts via devices)
  - Promotions/discounts engine — P14-01
  - Variant barcode mapping (scan → correct variant) — P11-02
  - Price/tax snapshot for replay integrity — P5-05
  - Crypto/QR tender — Proposed (via Integration Gateway)
- Special cases
  - Returns policy + manager override (with payment refund passthrough) — P3-02, P3-03, P6-03
  - Loyalty redemption + offline cache — P4-02, P4-04
  - BOPIS pickup workflow states — P8-02
- Closing / after sales
  - Offline orders/replay + sync layer (queue, diff, duplicate prevention) — P5-01, P5-02, P9-01, P9-02, P9-03
  - Cash drawer open/close, counting, and EOD settlement — Proposed
  - Tasking & store checklists for opening/closing — P17-01, P17-02
  - KPI/perf telemetry and POS offline queue metrics — P0-04, P0-06

### Critical Path (Cashier MVP Recommendation)

1) P13-01 Device SDK (printer/scanner/terminal) [cashier-addendum]
2) P6-01 Payment intents (+ P6-02 webhook hardening) [cashier-addendum]
3) P6-03 Refund passthrough (to support returns/exchanges refunds) [cashier-addendum]
4) P13-02 Hot plug detection and retries [cashier-addendum]
5) P16-01 E‑receipt templates OR ensure print-only receipts via P13-01 [cashier-addendum]
  Decision: For MVP, we chose print-only receipts using device printer; E‑receipts deferred to Phase 16.
6) P11-02 Variant barcode mapping (or accept base SKU scans initially) [cashier-addendum]
7) P3-02 Return policy and P3-03 manager override (with audit) [cashier-addendum]
8) P4-02 Loyalty redemption (optional for MVP; accrual already live) [cashier-addendum]
9) P5-01 Replay validation (+ P5-03 POS UI) if offline is day 1 [cashier-addendum]
10) P0-04 KPI instrumentation for checkout latency/taps [cashier-addendum]

### Proposed Additions (New Items — Append-only)

- Offline login grace window with cached credentials/claims
  - Actions: POS caches encrypted credentials/claims; configurable grace TTL; offline mode banner; audit offline enter/exit; tenant toggle.
  - Acceptance: Login and transact during outage within TTL; resync on reconnect without data loss; security review completed.
  - Dependencies: P2-01, P5-01 (if offline orders are day 1), P0-04.

- Cash tender + drawer lifecycle (open/close, counts, over/short, Z report)
  - Actions: Model cash tender; drawer sessions and counts; change calc; EOD Z report; audit; minimal report export.
  - Acceptance: Cash sales complete with change; drawer discrepancies tracked; EOD report available; audit/metrics present.
  - Dependencies: P13-01 (device SDK), P7-01 (roles), P6-01 (linkages), P16-01 or print pipeline.

- Crypto/QR tender integration
  - Actions: QR initiation in POS; Integration Gateway route; webhook confirmation in payment-service; refund passthrough.
  - Acceptance: Sandbox E2E settles; refunds supported; audit/telemetry captured; tenant opt-in.
  - Dependencies: P6-01, P6-02, P6-03, Integration Gateway.

### References

- POS Receipts (MVP): docs/development/pos-receipts.md
