# MVP Gaps Log

This document aggregates the gaps we have identified while reviewing the current NovaPOS implementation. It will evolve as new analyses land.

## -------------- Order Service (Sales Transactions)

### Current Implementation

- Rust microservice exposes REST endpoints
- Order creation now pre-reserves inventory via the inventory-service reservation API and rolls back holds on failure cases (`services/order-service/src/order_handlers.rs:358`, `services/inventory-service/src/reservation_handlers.rs:46`).
- Refund processing validates quantities, persists return records, and flips orders to partial/full refunded states while emitting audit events (`services/order-service/src/order_handlers.rs:713`, `services/order-service/migrations/2005_add_return_tracking.sql:1`). for create/list/refund and an offline clear helper (`services/order-service/src/order_handlers.rs:139`, `services/order-service/src/main.rs:123`).
- Order writes enforce `X-Tenant-ID`, persist payment method metadata, and accept optional idempotency keys to dedupe retries (`services/order-service/src/order_handlers.rs:140`, `services/order-service/migrations/2003_add_order_items_and_idempotency.sql:1`).
- Line items now persist to `order_items` with unit pricing, and events/refunds pull pricing from the database (`services/order-service/src/order_handlers.rs:111`, `services/order-service/src/order_handlers.rs:287`, `services/order-service/src/main.rs:170`).
- Managers can void in-flight orders via `POST /orders/:id/void`, which marks the sale `VOIDED` and emits an `order.voided` event for downstream cleanup (`services/order-service/src/order_handlers.rs:308`, `services/order-service/src/main.rs:131`).
- Card/crypto payments remain `PENDING` until Kafka confirmation; the consumer applies parameterised status updates and rehydrates payloads from storage instead of an in-memory cache (`services/order-service/src/main.rs:150`, `services/order-service/src/main.rs:170`).

### Order Service: Confirmed Gaps / Risks

- Operational tooling gaps persist: admin surfaces still lack deep order search, receipts, or return dashboards despite richer APIs (`services/order-service/src/order_handlers.rs:1017`, `frontends/admin-portal/src/pages/OrdersPage.tsx`).

### Order Service: Implications

- Cash and card experiences diverge, and PENDING orders stall if services reboot, blocking settlement and downstream loyalty/inventory sync.
- Ops teams lack the endpoints needed for voids, partial returns, or order history auditing.

## Checkout Speed & Offline-first Integrity

### Checkout Speed & Offline-first: Current Implementation

- POS keeps an offline queue in localStorage and retries submits when connectivity returns (`frontends/pos-app/src/OrderContext.tsx:23`, `frontends/pos-app/src/OrderContext.tsx:386`).
- Order records now carry payment method, persisted line items, and optional idempotency metadata to safeguard retries (`services/order-service/src/order_handlers.rs:69`, `services/order-service/src/order_handlers.rs:160`, `services/order-service/migrations/2003_add_order_items_and_idempotency.sql:1`).

### Checkout Speed & Offline-first: Confirmed Gaps / Risks

- Offline queue still generates submissions without deterministic `idempotency_key` values; frontend work remains to leverage the backend guard (`frontends/pos-app/src/OrderContext.tsx:386`, `services/order-service/src/order_handlers.rs:160`).
- Item-level data is stored server-side but UI reconciliation and receipt flows still rely on cached payloads (`frontends/pos-app/src/components/pos/ProductGrid.tsx:9`).

### Checkout Speed & Offline-first: Implications

- Cashiers risk duplicate taps; finance teams cannot audit offline adjustments vs. live catalog changes.

## -------------- Unified Tender & Payment Orchestration

### Unified Tender & Payment: Current Implementation

- Cash orders settle immediately; order-service marks non-card/crypto tenders as `COMPLETED` and emits `order.completed` inline (`services/order-service/src/order_handlers.rs:129`, `services/order-service/src/order_handlers.rs:181`).
- Card capture flows through Integration Gateway to the Payment Service Valor stub, then publishes `payment.completed`; no card PAN data is persisted, keeping PCI scope reduced for now (`services/integration-gateway/src/integration_handlers.rs:195`, `services/payment-service/src/payment_handlers.rs:36`).
- Crypto checkout supports Coinbase hosted charges or stub mode, returning a payment URL and depending on async events (internal timer or Coinbase webhooks) for completion (`services/integration-gateway/src/integration_handlers.rs:47`, `services/integration-gateway/src/webhook_handlers.rs:55`).

### Unified Tender & Payment: Confirmed Gaps / Risks

- Hardware terminal SDK integration is still outstanding; the Valor shim cannot handle chip fallback, signature capture, or device diagnostics (`services/payment-service/src/payment_handlers.rs:36`).
- Failure handling remains shallow: declines, timeouts, partial approvals, and reversals all collapse into generic gateway errors with no retry or escalation guidance (`services/integration-gateway/src/integration_handlers.rs:211`).
- Refunds and voids never reach the upstream gateway/crypto provider, so original-tender reversals and settlement adjustments will drift once real processors are attached (`services/order-service/src/order_handlers.rs:245`).
- Payments lack idempotency tokens, and Coinbase webhook verification does not guard against replay or key rotation, leaving duplicate/forged notifications possible (`services/integration-gateway/src/integration_handlers.rs:21`, `services/integration-gateway/src/webhook_handlers.rs:63`).

### Unified Tender & Payment: Implications

- POS cannot be deployed with production card hardware yet, and operations have no safety net for complicated tender scenarios.
- Weak idempotency and webhook posture risks mismatched ledger entries and support escalations when providers retry or attackers replay events.

## -------------- Admin Portal & Management Tools

### Admin Portal & Management: Current Implementation

- React admin portal routes expose dashboard, product, user, and settings screens without per-route guards (`frontends/admin-portal/src/App.tsx:14`).
- Product management page handles list/create/update/toggle flows and fetches per-product audit histories while attaching tenant headers on every call (`frontends/admin-portal/src/pages/ProductListPage.tsx:93`, `frontends/admin-portal/src/pages/ProductListPage.tsx:240`, `frontends/admin-portal/src/pages/ProductListPage.tsx:171`).
- Users view pulls role/tenant catalogs and submits new user records via the auth service but exposes only a simple create form driven by static fallback roles (`frontends/admin-portal/src/pages/UsersPage.tsx:23`, `frontends/admin-portal/src/pages/UsersPage.tsx:146`, `frontends/admin-portal/src/pages/UsersPage.tsx:249`).
- Settings view lets super admins create tenants, mint integration keys, and clear offline order queues by forwarding the selected tenant in headers (`frontends/admin-portal/src/pages/SettingsPage.tsx:220`, `frontends/admin-portal/src/pages/SettingsPage.tsx:232`, `frontends/admin-portal/src/pages/SettingsPage.tsx:329`).

### Admin Portal & Management: Confirmed Gaps / Risks

- No client-side RBAC guard: all navigation renders for any logged-in user and components rely on ad hoc conditional renders, so mid-tier roles can reach privileged forms (`frontends/admin-portal/src/App.tsx:14`, `frontends/admin-portal/src/pages/UsersPage.tsx:79`).
- User management stops at single user creation; there are no edit, deactivate, password reset, or audit flows for existing accounts (`frontends/admin-portal/src/pages/UsersPage.tsx:264`).
- Analytics dashboard is limited to a single-day summary; there are no historical filters, cohort views, or tenant switching for HQ oversight (`frontends/admin-portal/src/pages/DashboardPage.tsx:57`, `frontends/admin-portal/src/pages/DashboardPage.tsx:141`).
- Inventory oversight is absent entirely; the portal ships no low-stock, replenishment, or cost pages despite product CRUD (`frontends/admin-portal/src/App.tsx:14`).
- Tenant onboarding is a thin form that captures only a name and optional API key with no downstream device provisioning or policy templates (`frontends/admin-portal/src/pages/SettingsPage.tsx:232`, `frontends/admin-portal/src/pages/SettingsPage.tsx:245`).
- Audit visibility is scoped to product changes; user, payment, and tenant events have no surfaced history (`frontends/admin-portal/src/pages/ProductListPage.tsx:171`, `frontends/admin-portal/src/pages/UsersPage.tsx:249`).

### Admin Portal & Management: Implications

- Headquarters lacks a governed back office: privileged actions can leak to managers, and there is no lifecycle tooling for user or tenant onboarding.
- Missing analytics and inventory tooling prevent ops from spotting performance or stock issues without dropping to raw service calls.

## -------------- Integration Gateway (External Hub)

### Integration Gateway: Current Implementation

- Axum service exposes `/payments`, `/external/order`, and Coinbase webhook endpoints, each protected by shared auth middleware (`services/integration-gateway/src/main.rs:312`).
- Auth middleware accepts JWT bearer tokens or cached API keys, enforces tenant header alignment, and records usage for rate-limit metrics (`services/integration-gateway/src/main.rs:340`).
- Redis-backed rate limiter caps requests per key/tenant (configurable RPM), publishing alerts when bursts exceed thresholds (`services/integration-gateway/src/main.rs:223`, `services/integration-gateway/src/main.rs:390`).
- Keys load from Auth service tables and refresh periodically; usage tracker emits Kafka metrics for observability (`services/integration-gateway/src/main.rs:205`).
- `/payments` proxies to Payment Service, `/external/order` forwards orders to Order Service, and coinbase webhooks validate signatures before pushing payment events (`services/integration-gateway/src/integration_handlers.rs:195`, `services/integration-gateway/src/integration_handlers.rs:329`).

### Integration Gateway: Confirmed Gaps / Risks

- Connector surface stops at payments/orders; there are no adapters for ERP, loyalty, accounting, or future partner APIs (`services/integration-gateway/src/integration_handlers.rs:329`).
- Rate limiting remains per-instance Redis; there is no global coordination or circuit breaker for distributed deployments (`services/integration-gateway/src/rate_limiter.rs:17`).
- API keys are the only credential�no OAuth2, client certificates, or scoped JWT issuance for partners (`services/integration-gateway/src/main.rs:340`).
- Webhook security is limited to Coinbase HMAC; other integrations lack signature verification or replay prevention (`services/integration-gateway/src/webhook_handlers.rs:63`).
- Monitoring relies on Prometheus metrics; there is no dedicated dashboard, audit trail, or alert routing for partner interactions (`services/integration-gateway/src/main.rs:200`).
- Tenant attribution hinges on headers supplied by the caller; without signed tokens, partner mistakes can bleed traffic across tenants (`services/integration-gateway/src/main.rs:353`).

### Integration Gateway: Implications

- Scaling beyond payments will require significant groundwork: richer auth, broader connectors, and ops tooling before onboarding enterprise partners.
- Weak credentialing and limited visibility increase the blast radius if a key leaks or a partner overloads the gateway.

## -------------- POS Edge (Client App)

### POS Edge: Current Implementation

- React/Vite PWA routes drive the cashier workflow across login, catalog, cart, checkout, and order history views (`frontends/pos-app/src/App.tsx:6`, `frontends/pos-app/src/pages/CashierPage.tsx:298`).
- Multi-tenant sessions live in `AuthContext`, which hydrates tokens, enforces inactivity timeouts, and carries tenant IDs into API headers (`frontends/pos-app/src/AuthContext.tsx:12`, `frontends/pos-app/src/AuthContext.tsx:96`).
- Orders queue in localStorage when offline and resync automatically, with UI banners showing queued counts and retry status (`frontends/pos-app/src/OrderContext.tsx:149`, `frontends/pos-app/src/components/pos/OfflineBanner.tsx:8`).
- Product data caches per tenant and falls back to the cache when fetches fail, keeping active items available offline (`frontends/pos-app/src/hooks/useProducts.ts:24`, `frontends/pos-app/src/hooks/useProducts.ts:111`).
- Touch-first layouts, large tap targets, and idle timers optimize the cashier view for tablets and kiosks (`frontends/pos-app/src/pages/CashierPage.tsx:298`, `frontends/pos-app/src/pages/CashierPage.tsx:82`).

### POS Edge: Confirmed Gaps / Risks

- Delivery remains browser-only; the Vite PWA has no native shell or kiosk lockdown hooks for managed mobile deployments (`frontends/pos-app/package.json:6`).
- Offline catalog caching omits inventory counts and price overrides, so stock accuracy and pricing drift during outages (`frontends/pos-app/src/hooks/useProducts.ts:104`).
- Hardware integrations are missing�scanner, printer, and terminal routines are absent, leaving manual entry even when peripherals exist (`frontends/pos-app/src/components/pos/ProductGrid.tsx:9`, `frontends/pos-app/src/components/pos/SubmitSalePanel.tsx:20`).
- Payment capture assumes REST handoffs to the integration gateway; there is no pairing with in-lane terminals or tap-to-pay flows (`frontends/pos-app/src/OrderContext.tsx:420`).
- Product rendering eagerly maps entire catalogs with no virtualization or pagination, risking sluggish UI for large assortments (`frontends/pos-app/src/components/pos/ProductGrid.tsx:9`).
- Kiosk safety stops at soft idle timers; there is no forced re-auth, wake-lock control, or session pinning for unattended devices (`frontends/pos-app/src/pages/CashierPage.tsx:82`).

### POS Edge: Implications

- Store rollouts that rely on rugged tablets or native kiosk management will stall without deeper device support.
- Stale offline data and missing peripheral hooks drive manual workarounds precisely when connectivity is weakest, risking longer lines and data entry errors.

## -------------- Offline Sync Mechanism

### Offline Sync: Current Implementation

- `OrderContext` queues draft orders with temp IDs in localStorage, marks them `offline: true`, and exposes queue/retry helpers via context (`frontends/pos-app/src/OrderContext.tsx:149`, `frontends/pos-app/src/OrderContext.tsx:714`).
- `useSyncOnReconnect` listens for the browser `online` event and triggers queue flushes once connectivity returns (`frontends/pos-app/src/hooks/useSyncOnReconnect.ts:5`).
- When flush succeeds, orders are re-posted to the order service, payment attempts are retried through the integration gateway, and recent history entries are rewritten with real IDs (`frontends/pos-app/src/OrderContext.tsx:415`).
- The order service persists an `offline` flag and exposes a clear endpoint so admins can purge unsynced records (`services/order-service/src/order_handlers.rs:152`, `services/order-service/src/order_handlers.rs:282`).

### Offline Sync: Confirmed Gaps / Risks

- Queue flush only validates items client-side; there is no server reconciliation to catch price changes or depleted inventory before resubmitting (`frontends/pos-app/src/OrderContext.tsx:388`).
- Multi-device conflicts are unhandled�if two registers sell the last unit while offline, the second sale is accepted without stock awareness (`frontends/pos-app/src/hooks/useProducts.ts:104`).
- Error handling after failed syncs is limited to banner text; there is no duplicate detection, operator resolution workflow, or escalation hints (`frontends/pos-app/src/components/pos/OfflineBanner.tsx:8`).
- Idempotency relies on temp IDs in localStorage, but requests sent after reconnect lack idempotency keys, risking duplicate orders if the network flaps mid-submit (`frontends/pos-app/src/OrderContext.tsx:337`).
- Sync telemetry stops at console warnings; there is no centralized log of retries, failures, or queue depth for ops to monitor (`frontends/pos-app/src/OrderContext.tsx:451`).

### Offline Sync: Implications

- Inventory and pricing can drift silently after outages, creating reconciliation work and customer-facing errors.
- Without conflict resolution and visibility, store teams must guess which queued orders truly posted, increasing duplicate transactions and support load.

## -------------- Security & Compliance (Multi-Tenancy & Auth)

### Security & Compliance: Current Implementation

- Auth service issues RS256 access tokens and hashed refresh tokens, and every microservice verifies them via the shared `JwtVerifier` extractor (`services/auth-service/src/tokens.rs:234`, `services/common/auth/src/extractors.rs:14`, `services/order-service/src/main.rs:36`).
- Tenant context is carried end-to-end: handlers compare `X-Tenant-ID` headers to JWT claims before querying tenant-scoped rows (`services/order-service/src/order_handlers.rs:34`, `services/product-service/src/product_handlers.rs:72`).
- Credentials are stored with Argon2 hashes, five-attempt lockouts, and optional TOTP MFA during login (`services/auth-service/src/user_handlers.rs:31`, `services/auth-service/src/user_handlers.rs:508`, `services/auth-service/src/user_handlers.rs:685`).
- Customer service encrypts PII with per-tenant data keys and exposes GDPR export/delete endpoints that record tombstones (`services/customer-service/src/main.rs:231`, `services/customer-service/src/main.rs:427`, `services/customer-service/src/migrations/5004_create_gdpr_tombstones.sql:1`).
- Payment flows keep PCI scope narrow by brokering card traffic through the gateway + stubbed terminal handler rather than persisting PAN data (`services/integration-gateway/src/integration_handlers.rs:195`, `services/payment-service/src/payment_handlers.rs:36`).

### Security & Compliance: Confirmed Gaps / Risks

- Tenant enforcement is hand-coded in every handler and trusts client headers; missing a `tenant_id_from_request` call would leak cross-tenant data, and there is no fallback to claims or database row-level security (`services/product-service/src/product_handlers.rs:72`).
- RBAC remains ad hoc; some read endpoints skip role checks entirely (for example product listing), so any authenticated role can enumerate catalog data (`services/product-service/src/product_handlers.rs:321`).
- Audit coverage is limited to product CRUD; order, payment, auth, and customer operations do not emit structured security events today (`services/product-service/src/product_handlers.rs:149`).
- GDPR tooling lives only in customer-service; orders, loyalty, and analytics retain personal identifiers without aligned retention or erasure workflows (`services/customer-service/src/main.rs:703`).
- Every backend exposes its port directly in docker-compose, so there is no gateway choke point or network segmentation to contain compromised services (`docker-compose.yml:92`).
- Security monitoring is mostly metrics; there is no centralized alerting for failed logins, role escalations, or tenant boundary violations beyond local logs (`services/auth-service/src/user_handlers.rs:453`).

### Security & Compliance: Implications

- A missed tenant or role check could leak another tenant�s data, undermining compliance commitments and trust.
- Without unified auditing and monitoring, incident response and regulatory reporting will stall once the system goes live.

## -------------- Inventory Service (Stock Management)

### Inventory Service: Current Implementation

- Kafka consumer listens to `order.completed`, `product.created`, and `payment.completed` topics, auto-seeding records and logging payments as no-ops (`services/inventory-service/src/main.rs:92`).
- `handle_order_completed` decrements tenant-level quantity per product and emits `inventory.low_stock` when counts fall below the configured threshold (`services/inventory-service/src/main.rs:171`, `services/inventory-service/src/main.rs:242`).
- `handle_product_created` seeds inventory rows with optional initial quantity/threshold, writing rows only when missing (`services/inventory-service/src/main.rs:262`).
- REST `GET /inventory` enforces tenant headers + JWT claims and returns current quantity/threshold per product (`services/inventory-service/src/inventory_handlers.rs:12`, `services/inventory-service/src/inventory_handlers.rs:40`).
- Schema tracks quantity and threshold at product+tenant level, providing a default low-stock trigger (`services/inventory-service/migrations/4001_create_inventory.sql:1`).

### Inventory Service: Confirmed Gaps / Risks

- Stock is global per tenant; there is no location or channel dimension, so multi-store accuracy is impossible (`services/inventory-service/src/main.rs:171`).
- Pre-sale validation is absent�orders are decremented after completion, so oversells occur when inventory is tight (`services/inventory-service/src/main.rs:171`).
- No admin APIs exist for manual adjustments, transfers, or CSV bulk updates, forcing DB edits for corrections (`services/inventory-service/src/inventory_handlers.rs:12`).
- Low-stock alerts land only on Kafka; nothing surfaces them in the portal or via email/push today (`services/inventory-service/src/main.rs:242`, `frontends/admin-portal/src` lacking listeners).
- History tables and reconciliation tooling are missing, making investigations of shrink or returns impossible (`services/inventory-service/src/main.rs:171`).
- Event semantics between `order.completed` and `payment.completed` remain ambiguous, risking double adjustments or missed decrements when states evolve (`services/inventory-service/src/main.rs:142`).

### Inventory Service: Implications

- Without per-location stock and proactive validation, MVP stores will oversell and lack audit trails when discrepancies appear.
- Ops cannot respond quickly to low-stock or reconcile adjustments, leaving replenishment and financial accuracy at risk.

## Returns & Exchanges Engine

### Returns & Exchanges: Current Implementation

- Refund endpoint flips order status to `REFUNDED` and emits a compensating negative `order.completed` event (`services/order-service/src/order_handlers.rs:200`).
- Inventory Service exposes read-only stock queries with no restock actions (`services/inventory-service/src/inventory_handlers.rs:74`).

### Returns & Exchanges: Confirmed Gaps / Risks

- Returns now record per-line adjustments and enforce quantity limits, but policy workflows (restock rules, fees, manager overrides) and exchange flows are still missing (`services/order-service/src/order_handlers.rs:713`).

### Returns & Exchanges: Implications

- Store teams cannot enforce policy-aware returns; inventory accuracy degrades when refunds occur.

## -------------- Loyalty & Customer 360

### Loyalty & Customer 360: Current Implementation

- Loyalty Service listens to `order.completed` and applies a floor(USD) earn rule, updating points and emitting `loyalty.updated` (`services/loyalty-service/src/main.rs:53`).
- Customer Service stores encrypted profiles and supports hashed email/phone lookup (`services/customer-service/src/main.rs:189`).

### Loyalty & Customer 360: Confirmed Gaps / Risks

- No redemption or tiering logic; offline balance cache is absent.
- POS cannot surface consolidated customer profile or loyalty balance while offline.

### Loyalty & Customer 360: Implications

- Shoppers see limited loyalty value at checkout; marketing cannot drive personalized offers from real-time data.

## BOPIS & Inventory Promise

### BOPIS & Inventory Promise: Current Implementation

- Inventory endpoints expose current quantity snapshots (`services/inventory-service/src/inventory_handlers.rs:74`).

### BOPIS & Inventory Promise: Confirmed Gaps / Risks

- No reservation or order-promise layer; pickup orders could oversell.

### BOPIS & Inventory Promise: Implications

- Click-and-collect experiences remain speculative; inventory accuracy suffers under demand spikes.

## Cross-cutting Observations

- Tenancy enforcement is duplicated per service; shared middleware would reduce drift (`services/order-service/src/order_handlers.rs:34`, `services/inventory-service/src/inventory_handlers.rs:26`).
- REST APIs lack versioning and idempotency headers; webhook retry semantics remain undefined (`services/integration-gateway/src/integration_handlers.rs:39`).
- Kafka workflows run fire-and-forget without sagas or compensating actions, increasing recovery complexity (`services/order-service/src/order_handlers.rs:210`).
- Order timestamps default to UTC `NOW()` with no store-local boundary controls, risking reporting drift (`services/order-service/migrations/2002_extend_orders.sql:1`).
