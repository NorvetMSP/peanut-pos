# NovaPOS MVP Gap Build Tracker

> Living document consolidating current implementation status vs. strategic MVP gap closure. Source-of-truth for engineering planning; kept concise & actionable. Update incrementally per PR.

## 1. Purpose

Provide an always-current snapshot of: (a) feature gap validation, (b) remaining prioritized work, (c) near-term sprint focus. Replaces ad‑hoc spreadsheets and diffused notes.

## 2. Scope

Covers domains A–K spanning Orders, Payments, Admin, Security/Compliance, Inventory, Returns/Exchanges, Loyalty, POS Edge, Fulfillment (BOPIS), Offline Sync, Cross-Cutting Architecture.

Related docs:

- `doc/analysis/MVP_Gaps.md` (original gap enumeration)
- `doc/analysis/Implementation_Plan_for_Addressing_NovaPOS_MVP_Gaps.md` (earlier narrative plan)
- `docs/security/security_plan.txt` (security + audit design drafts)

## 3. Status Matrix (High-Level)

Legend: ✅ Done | 🌓 Partial | ⛔ Missing

| Domain | Capability | Status | Evidence (file:line or area) | Notes / Next Action |
|--------|------------|--------|------------------------------|---------------------|
| A Orders/Offline | Offline queue + idempotency key (client) | 🌓 | `frontends/pos-app/src/OrderContext.tsx` | Needs server replay validation & diff UX |
| A Orders/Offline | Replay validation (price/inventory) | ⛔ | — | Add validation endpoint in order-service referencing inventory-service |
| A Orders/Offline | Stalled PENDING watchdog/reconciliation | ⛔ | — | Background job + metrics |
| A Orders/Offline | Historical exports / analytics breadth | ⛔ | — | Export API (CSV/JSON) + aggregation views |
| A Orders/Offline | Offline conflict resolution UI | ⛔ | — | Modal diff compare UX |
| B Payments | Physical terminal SDK integration | ⛔ | — | Abstract driver + device handshake |
| B Payments | Failure taxonomy & reversals | ⛔ | — | Enum + state machine + reversal endpoints |
| B Payments | Refund/reversal passthrough | ⛔ | — | Payment intent + refund table |
| B Payments | Idempotent payment intent model | ⛔ | — | `payment_intents` schema |
| B Payments | Webhook signature + replay defense | ⛔ | — | HMAC middleware + nonce storage |
| B Payments | Partner connectors / scoped creds | ⛔ | — | Credential schema + rotation |
| C Admin | User lifecycle CRUD & roles | 🌓 | `auth-service/*` (tenants, keys) | Missing full user mgmt UI/actions |
| C Admin | Analytics HQ rollups | ⛔ | — | Multi-tenant aggregation views |
| C Admin | Inventory oversight UI | ⛔ | — | Low-stock list, adjustments history |
| C Admin | Tenant onboarding workflow | 🌓 | `auth-service/src/tenant_handlers.rs` | Need provisioning hooks |
| C Admin | Global audit views | ⛔ | — | Audit search API + UI |
| D Security | Central tenancy middleware | 🌓 | Duplicate guards across services | Consolidate into shared crate |
| D Security | Consistent RBAC enforcement | 🌓 | Product/customer handlers | Audit coverage uneven |
| D Security | Unified audit pipeline | 🌓 | `docs/security/security_plan.txt` | Wire producer/consumer across services |
| D Security | GDPR/retention (non-customer) | ⛔ | — | Extend tombstones, purge jobs |
| D Security | Network segmentation | ⛔ | `docker-compose.yml` flat network | Define ingress/egress profiles |
| D Security | Timezone/reporting strategy | ⛔ | — | Store tz + conversion utils |
| E Inventory | Per-location inventory model | ✅ | `inventory-service` migrations 4003–4005; code in `inventory_handlers.rs` | Multi-location tables + aggregation queries implemented |
| E Inventory | Reservation lifecycle (basic) | ✅ | `reservation_handlers.rs`, sweeper in `main.rs` | TTL + expiration sweeper + audit + Kafka events |
| E Inventory | Adjustment & transfer APIs | ⛔ | — | New endpoints + audit (not started) |
| E Inventory | Low-stock alerts & audit | 🌓 | `main.rs` low_stock emission; audit events in sweeper | UI & threshold mgmt panel pending |
| E Inventory | Event dedupe semantics | ⛔ | — | Idempotent keys / hashes |
| F Returns | Basic return initiation UI | 🌓 | `admin-portal/ReturnsPage.tsx` | Backend policies missing |
| F Returns | Policy module (fees, conditions) | ⛔ | — | `return_policies` table |
| F Returns | Exchange flow | ⛔ | — | Replacement order delta logic |
| F Returns | Manager override + audit | ⛔ | — | Role check + audit event |
| F Returns | Tender reversal passthrough | ⛔ | — | Gateway integration stub |
| G Loyalty | Points read endpoint | ✅ | `loyalty-service/src/main.rs` | Redemption absent |
| G Loyalty | Redemption / burn / expiry | ⛔ | — | Transactional decrement |
| G Loyalty | Tiering & promotions | ⛔ | — | Tier rules engine |
| G Loyalty | Offline cache conflict handling | ⛔ | — | Snapshot + reconcile logic |
| G Loyalty | POS customer 360 view | ⛔ | — | Unified composite endpoint/UI |
| H POS Edge | Peripheral integrations | ⛔ | — | Scanner/printer abstractions |
| H POS Edge | Large catalog virtualization | ⛔ | — | Introduce windowed list |
| H POS Edge | Kiosk lockdown & session hardening | ⛔ | — | Idle timeout + focus traps |
| H POS Edge | Native/device mgmt wrappers | ⛔ | — | Device registration + health |
| I BOPIS | Reservation/promise layer | 🌓 | Basic reservation endpoints | Add expirations & statuses |
| I BOPIS | Pickup workflow states | ⛔ | — | Pickup domain model |
| J Offline Sync | Conflict diff service | ⛔ | — | Server diff endpoint |
| J Offline Sync | Retry telemetry & monitoring | ⛔ | — | Metrics + dashboard |
| J Offline Sync | Duplicate prevention beyond idempotency | ⛔ | — | Content hash/store |
| K Cross-Cutting | API versioning standard | ⛔ | — | Adopt `/v1` routing pattern |
| K Cross-Cutting | Idempotency header contract | ⛔ | — | Spec + persistence |
| K Cross-Cutting | Saga/compensation orchestration | ⛔ | — | Outbox or workflow engine |
| K Cross-Cutting | Unified timezone/reporting boundary | ⛔ | — | Shared lib (dup with D) |

> NOTE: Evidence placeholders “—” indicate no code artifacts located via search; confirm before marking as Missing in production tracking.

## 3.1 Detailed Validation Narrative (Status vs Evidence)

Legend: Done = implemented & exercised, Partial = foundations exist but gaps / breadth missing, Missing = no substantive code or only planning docs.

### A. Orders / Offline / Analytics

- Offline order queue & idempotency key generation: Partial (client logic in `OrderContext.tsx` lines ~21–120 & 918+; lacks server-side reconciliation / conflict resolution).
- Resume & reconciliation jobs (pending→completed stall detection): Missing (no background job / cron logic found).
- Duplicate/conflict detection UX (offline replay): Missing (no diff UI components).
- Historical / cohort / multi-day exports: Missing (no analytics export endpoints).
- Server-side replay validation (price/inventory): Missing (no validation layer in order-service referencing inventory delta; only inventory events consuming order topics).

### B. Payments & Gateway

- Physical terminal SDK integration: Missing (no terminal/peripheral code).
- Failure taxonomy / partial approvals / reversals: Missing (no structured enums or retry orchestrations in payment-service).
- Refund & reversal passthrough (card/crypto): Missing (no refund endpoints).
- Idempotency tokens for payment intents: Missing (only frontend idempotency key in orders; no `payment_intent` table).
- Webhook (replay/mutation) hardening: Missing (integration-gateway has API key auth & rateLimiter, but no webhook signature verification beyond Coinbase placeholder).
- Expanded connectors & credential scoping (OAuth2/JWT per connector): Missing (only integration keys in auth-service `tenant_handlers.rs`).
- Partner dashboards & alerting: Missing (no UI pages or routes).

### C. Admin Portal / Management

- User lifecycle (edit/deactivate/role changes): Partial (auth-service tenant & integration key mgmt exists; UI lacks full user admin CRUD).
- Expanded analytics / HQ rollups: Missing.
- Inventory oversight UI (low-stock, adjustments history): Missing (only product list & orders/returns pages).
- Tenant onboarding workflow: Partial (tenant creation in `tenant_handlers.rs`, no provisioning hooks).
- Global audit views: Missing (audit ingestion documented in `security_plan.txt`, no UI).

### D. Security & Compliance

- Central tenancy middleware: Partial (duplicate per-service `tenant_id_from_request`; see analysis doc calling out duplication).
- Consistent RBAC enforcement: Partial (product and customer handlers enforce roles; uneven across all services).
- Unified structured audit pipeline: Partial (design + code fragments for audit producer/consumer in `security_plan.txt`, not fully integrated into services).
- GDPR/retention beyond customer-service: Missing (GDPR operations only in customer-service).
- Network segmentation / ingress hardening: Missing (compose uses flat network; no sidecar/ingress rules).
- Timezone/local-time reporting strategy: Missing (no timezone normalization beyond naive `Utc::now()`).

### E. Inventory

- Per-location inventory model: Done. New tables `locations`, `inventory_items` added via migrations (4003_create_locations.sql, 4004_create_inventory_items_multilocation.sql, 4005_backfill_inventory_items.sql) with dual-write validation logic and aggregated query paths in `inventory_handlers.rs`.
- Reservation lifecycle: Done. Create & release endpoints plus expiration sweeper (periodic task in `main.rs`) enforcing TTL via `expires_at` and emitting both domain (`inventory.reservation.expired`) and audit (`audit.events`) Kafka messages; integration test (`multilocation_lifecycle.rs`) validates restock + events.
- Low-stock alerts & audit: Partial. Emission of `inventory.low_stock` events implemented after order completion when quantity <= threshold. UI surfacing, threshold management UX, and alert tuning still pending.
- Adjustment & transfer APIs: Missing (no routes for manual adjustments or inter-location transfers yet).
- Event dedupe semantics: Missing (no idempotency keys / hash-based suppression on emitted events beyond Kafka topic semantics).

### F. Returns & Exchanges

- Returns initiation UI: Partial (`ReturnsPage.tsx` fetching returns, initiating basic return).
- Policy module (restock fees, condition codes): Missing (no schema for return policies).
- Exchange flow: Missing.
- Manager override & audit: Missing.
- Gateway passthrough for original tender reversals: Missing.

### G. Loyalty & Customer Programs

- Points accrual basic read: Done (loyalty-service `/points`, SQL query file).
- Redemption engine (burn/partial/expiry): Missing.
- Tiering rules & promotion hooks: Missing.
- Offline loyalty balance caching/conflict resolution: Missing.
- Consolidated customer 360 view at POS: Missing (POS does not surface loyalty/ recent orders integrated view; only order state).

### H. POS Edge (Device & Performance)

- Peripheral integrations (scanner/printer/terminal/cash drawer): Missing.
- Large catalog performance optimizations (virtualized grid/search indexing): Missing (no virtualization libs; simple React pages).
- Kiosk lockdown (session pinning, idle policies): Missing.
- Native shell / device mgmt wrappers: Missing.

### I. BOPIS / Fulfillment

- Reservation/promise layer (time-bound): Partial (basic reservation create/release; lacks expiry, SLA).
- Pickup workflow states & reconciliation: Missing (no pickup status model).

### J. Offline Sync Layer

- Conflict detection (price/inventory mismatch) UI: Missing.
- Retry telemetry / monitoring (queue depth, failure reasons): Missing (local storage only).
- Duplicate prevention beyond idempotency (content hash / diff): Missing.

### K. Cross-cutting Architecture

- API versioning standard: Missing (no `/v1/` prefixes across all services consistently).
- Standard idempotency header contract: Missing (client-side only, no server spec).
- Saga / compensation orchestration: Missing (no orchestrator/outbox tables; fire-and-forget events).
- Unified timezone/reporting boundary: Missing (duplicate with D).

## 4. Consolidated Remaining Work (Prioritized Themes)

(Keep this section pruned; move delivered items to a change-log entry below.)

1. Multi-Location Inventory & Reservations
2. Payment Intent Model & Webhook Hardening
3. Offline Order Replay Validation & Conflict UX
4. Unified Tenancy + RBAC + Audit Integration
5. Returns Policy & Exchange Framework
6. Loyalty Redemption + Tier Engine
7. BOPIS & Pickup Lifecycle
8. Offline Sync Telemetry & Dedup
9. Peripheral & Performance (POS Edge) Foundations
10. Cross-Cutting: API Versioning, Idempotency, Saga Skeleton, Timezone Handling

## 5. Next Sprint Starter Epics (Initial Story Slices)

### Epic: Inventory Multi-Location Foundation

- Design & migration: add `locations`, augment `inventory_items(location_id)`, backfill. — ✅ Completed (migrations 4003–4005 applied; backfill script logic executed during startup/tests).
- Update reservation endpoints to accept `location_id`; add expiration job. — ✅ Completed (handlers updated; sweeper task with TTL + audit + restock implemented).
- Publish low-stock event prototype. — 🌓 Implemented event emission to `inventory.low_stock`; pending admin/UI consumption & threshold management tooling.

### Epic: Tenancy & Audit Unification

- Extract shared tenancy/RBAC crate; apply to 2 pilot services.
- Introduce audit producer wrapper; instrument product + order mutations.
- Add global audit search backend route scaffold.

### Epic: Offline Replay Validation

- Define diff contract (incoming vs authoritative) + conflict codes.
- Order-service validation endpoint (inventory/price checks).
- POS modal for conflict resolution (adjust, override, cancel).

### Epic: Payment Intent & Webhook Hardening

- `payment_intents` table (idempotency_key unique, status enum).
- Intent creation API + state transitions (requires_capture, succeeded, failed, reversed).
- HMAC webhook middleware + delivery log + replay detection.

### Epic: Returns Policy Module (Phase 1)

- `return_policies` schema (restock_fee, condition_codes JSON, approval_threshold).
- Apply policy in return calculation endpoint + UI fee display.
- Audit events on create/update usage.

(Defer: Loyalty, BOPIS, Saga infrastructure until foundations stable.)

## 5.1 Expanded Action-Oriented Backlog (Detailed)

### Offline & Orders

1. Implement server-side order replay validation (price + inventory re-check) with diff response contract.
2. Add stalled PENDING order watchdog + reconciliation job (Kafka or scheduled).
3. Build offline conflict resolution UI (diff before commit).
4. Introduce historical order/settlement export endpoints (date range, CSV/JSON).

### Payments & Gateway

1. Define `payment_intent` schema (idempotent key, status transitions, reversal link).
2. Implement failure taxonomy & mapping (timeout, partial_approval, declined, network_error).
3. Add webhook signature verification + replay detection (nonce + expiry).
4. Refund / reversal stub endpoints (card + crypto abstraction).
5. Connector credential model (scoped token per partner) + rotation.

### Admin & Management

1. User management CRUD (roles, deactivate, reset) with audit events.
2. Inventory oversight page (low-stock list, adjustment history placeholder).
3. Tenant onboarding hook pipeline (seed roles, default policies).
4. Global audit search UI (paged, filter by actor, action, severity).

### Security & Compliance

1. Shared tenancy + RBAC middleware crate reused across services (remove duplicates).
2. Expand audit producer integration into all mutating endpoints.
3. GDPR retention tasks for orders, loyalty, analytics (tombstone schema updates).
4. Timezone normalization (store tz column + reporting conversion utilities).
5. Network segmentation plan (compose profiles / future ingress config).

### Inventory & Fulfillment

1. Location-aware inventory schema migration (`inventory_items`: tenant_id, product_id, location_id, quantity).
2. Reservation expiration & release job + reason codes.
3. Adjustment & transfer APIs (bulk + audit events).
4. Low-stock threshold config + alert emission (Kafka topic).
5. Event dedupe strategy (idempotent keys on inventory adjustments).

### Returns & Exchanges

1. Return policy schema (restock_fee %, condition_code enum, approval_threshold).
2. Exchange workflow endpoints (reprice delta + inventory adjustments).
3. Manager override flow (role check + audit).
4. Payment reversal passthrough integration stub.

### Loyalty

1. Redemption / burn API (atomic points decrement) + expiry model.
2. Tier rules engine (thresholds, multipliers).
3. Offline loyalty cache (signed snapshot + conflict merge logic).
4. POS unified customer 360 panel (orders + loyalty + basic profile).

### POS Edge & Performance

1. Peripheral abstraction layer (scanner, printer interfaces).
2. Virtualized product catalog list (windowing) + indexed search.
3. Kiosk session + idle timeout enforcement.
4. Device management primitives (device registration, heartbeat).

### BOPIS / Fulfillment

1. Promise/hold model (expiration timestamp, status transitions).
2. Pickup workflow states & API (ready, picked_up, expired, cancelled).
3. Inventory reconciliation on pickup or expiration events.

### Offline Sync Layer

1. Conflict diff service endpoint (compare submitted vs authoritative values).
2. Retry telemetry metrics endpoint (queue depth, failure counts).
3. Content-hash duplicate suppression for offline queue.

### Cross-Cutting Architecture

1. Introduce uniform API versioning (`/v1`) + deprecation guidelines.
2. Standard Idempotency-Key header contract + persistence.
3. Saga orchestrator or outbox pattern introduction (initial for returns & exchanges).
4. Timezone & reporting boundary library (shared utilities).

## 6. Update Conventions

- Each merged PR touching a line item adds a bullet under Change Log with: date, PR/ref, capability, status change (e.g., ⛔→🌓 or 🌓→✅) and brief note.
- Status progression rules: Partial → Done only after prod-ready code + tests + docs reference.
- If capability descopes, annotate with strike-through and rationale.

## 7. Change Log

| Date | PR/Ref | Capability | Change | Notes |
|------|--------|-----------|--------|-------|
| 2025-10-01 | INIT | Document created | — | Baseline statuses captured |
| 2025-10-01 | INV-ML-1 | Per-location inventory model | ⛔→✅ | Migrations + handlers + aggregation queries |
| 2025-10-01 | INV-ML-2 | Reservation lifecycle (expiration) | 🌓→✅ | TTL + sweeper + audit + events |
| 2025-10-01 | INV-ML-3 | Low-stock alerts & audit | ⛔→🌓 | Event emission implemented; UI pending |

## 8. Open Questions / Decisions To Record

## 9. Immediate Technical Dependencies / Sequencing Notes

- Inventory multi-location migration should precede reservation expiration logic to avoid dual rewrites.
- Payment intent schema should land before refund/reversal stories to prevent churn.
- Tenancy middleware refactor early reduces duplication before adding more endpoints (returns / exchanges).
- Audit integration should accompany each new mutation endpoint for consistent coverage.
- Offline replay validation depends on inventory normalization (price + availability checks).

## 10. Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Large inventory schema migration | Downtime / data inconsistency | Shadow tables + phased cutover behind feature flag |
| Middleware refactor regression | Auth / tenancy breaks | Incremental rollout w/ integration tests, canary services |
| Payment intent introduction churn | Double work / partial adoption | Dual-path compatibility for one sprint, telemetry gating |
| Offline validation UX complexity | Delayed adoption | Ship read-only diff first, add override later |
| Lack of audit coverage | Forensics gaps | Establish audit checklist pre-merge |

## 11. Suggested Metrics to Track

- Offline replay conflict rate (% of offline orders needing adjustment).
- Payment intent duplicate rate (should approach zero).
- Average reservation expiration reclaim time.
- Audit coverage ratio (# audited mutating endpoints / total mutating endpoints).
- Inventory stock drift incidents (detected vs resolved).

(Keep small; open items either resolved or promoted to backlog story.)

- Payment failure taxonomy naming (pending workshop).
- Saga framework selection (Temporal vs bespoke outbox + lightweight orchestrator).
- Timezone source of truth (store-level tz vs tenant-level default).

---
_This file is living. Optimize for clarity + delta readability; avoid narrative sprawl._
