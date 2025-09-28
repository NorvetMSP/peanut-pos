# Journey Maps World Class Redesign Analysis

## Overview

- Reviewed the current NovaPOS codebase to map the practical engineering plan to real implementation gaps. Focus is on what exists today, risks against MVP KPIs, persona impact, and acceptance-test coverage.

## 1. Checkout Speed & Offline-first Integrity

- **Current footing:** The POS keeps an offline queue in browser localStorage and retries when connectivity returns (`frontends/pos-app/src/OrderContext.tsx:23`, `frontends/pos-app/src/OrderContext.tsx:293`, `frontends/pos-app/src/OrderContext.tsx:386`). The Order Service stores orders with only id, tenant, total, status, and an offline flag (`services/order-service/src/order_handlers.rs:68`, `services/order-service/src/order_handlers.rs:152`, `services/order-service/migrations/2001_create_orders.sql:1`). Pending card/crypto orders are tracked in an in-memory HashMap that is lost on restart (`services/order-service/src/main.rs:35`).
- **Gaps & risks:** No idempotency keys, no outbox/inbox pattern, and no persisted state machine; a retry can create duplicate sales, and a service crash drops pending transactions (`services/order-service/src/order_handlers.rs:74`, `services/order-service/src/main.rs:35`). Order items do not capture unit pricing or price-lock metadata, so mid-cart price changes cannot be reconciled (`services/order-service/src/order_handlers.rs:68`).
- **Persona impact:** Cashiers still face double-tap risk and unclear offline status messaging; shoppers may see mismatched totals if prices change while offline.
- **Acceptance coverage:** Only a happy-path offline flush test exists (`frontends/pos-app/src/OrderContext.test.tsx:33`); no stress tests for double taps, long-duration offline queues, or price-lock guarantees.

## 2. Unified Tender Service & Payment State Machine

- **Current footing:** Integration Gateway accepts a single payment method per order and proxies directly to stubbed card/crypto handlers, emitting a flat paid/pending status (`services/integration-gateway/src/integration_handlers.rs:16`, `services/integration-gateway/src/integration_handlers.rs:39`, `services/integration-gateway/src/integration_handlers.rs:266`). The Payment Service card handler is a two-second stub that always approves and returns an "approval_code" string (`services/payment-service/src/payment_handlers.rs:26`).
- **Gaps & risks:** There is no orchestrated state machine, no split tender support, no capture/refund lifecycle, and no persistence of gateway references for original-tender refunds (`services/order-service/src/order_handlers.rs:210`). Crypto flows rely on stub timeouts without webhook reconciliation.
- **Persona impact:** Cashiers cannot mix tenders or recover from payment errors gracefully; finance/audit teams lack unified payment records; shoppers cannot rely on original-tender refunds.
- **Acceptance coverage:** No integration or contract tests for multi-tender scenarios, refunds to original tender, or duplicate-prevention in payment retries.

## 3. Returns & Exchanges Engine

- **Current footing:** The Order Service exposes a simple refund endpoint that flips an order status to REFUNDED and emits a negative completion event (`services/order-service/src/order_handlers.rs:98`, `services/order-service/src/order_handlers.rs:210`). Inventory Service only supports listing stock (`services/inventory-service/src/inventory_handlers.rs:74`).
- **Gaps & risks:** There is no dedicated Return domain, no linkage to original order lines, no restock rules, fees, or fraud flags, and no manager approval workflow.
- **Persona impact:** Cashiers and shoppers cannot execute policy-aware returns or exchanges; inventory teams lose visibility into restocked items; managers cannot enforce thresholds.
- **Acceptance coverage:** No automated tests around returns, restocking, or cross-channel flows.

## 4. Loyalty & Customer 360

- **Current footing:** Loyalty Service listens to `order.completed` events and applies a floor(USD) earn rule, updating a points table and emitting a `loyalty.updated` message (`services/loyalty-service/src/main.rs:53`, `services/loyalty-service/src/main.rs:238`). Customer Service offers encrypted customer storage and search by hashed email/phone but returns only basic profile fields (`services/customer-service/src/main.rs:189`, `services/customer-service/src/main.rs:315`).
- **Gaps & risks:** No redemption, tiering, configurable earn/burn rates, or offline balance cache; POS cannot display holistic customer data or loyalty balances when offline; compensation flows for offline earn/burn are absent.
- **Persona impact:** Shoppers do not see loyalty value at checkout; managers lack a customer 360 view for personalized service; marketing cannot drive targeted offers from events.
- **Acceptance coverage:** No tests validating point accrual reversal, redemption limits, or stale-cache compensation.

## 5. BOPIS & Inventory Promise

- **Current footing:** Inventory Service exposes read-only quantities with thresholds (`services/inventory-service/src/inventory_handlers.rs:74`).
- **Gaps & risks:** No reservation/promise service, pick-pack workflows, SLA timers, or no-show releases; POS has no visibility into reserved stock.
- **Persona impact:** Shoppers risk oversold pickup orders; store associates lack workflow tooling; managers cannot monitor fulfillment queues.
- **Acceptance coverage:** None for BOPIS reserve, ready, or release scenarios.

## 6. Product Catalog Variants & Serials

- **Current footing:** The product schema stores only basic product attributes (`services/product-service/migrations/1001_create_products.sql:1`) and the API exposes simple CRUD without variant matrices or serial tracking (`services/product-service/src/product_handlers.rs:241`).
- **Gaps & risks:** No support for size/color variations, barcode-to-variant mapping, or serial-number capture; inventory per variant and returns of serialised goods cannot be enforced.
- **Persona impact:** Stores cannot sell apparel or electronics accurately; returns teams cannot validate serialised goods; HQ cannot manage complex catalogs.
- **Acceptance coverage:** No tests for variant or serial flows.

## 7. Manager Mobility & Approvals

- **Current footing:** No approval service exists; the only "approval" concept is a payment approval code string from the Valor stub (`services/payment-service/src/payment_handlers.rs:23`).
- **Gaps & risks:** No mobile approval tokens, no policy DSL, and no audit trail linking decisions to transactions.
- **Persona impact:** Cashiers still depend on in-person overrides; managers lack remote exception handling; audit/compliance cannot trace approvals.
- **Acceptance coverage:** None for approval paths.

## 8. Edge Device & Peripherals Layer

- **Current footing:** POS logic only monitors network reachability (`frontends/pos-app/src/OrderContext.tsx:273`) and has no abstraction for printers, scanners, or payment terminals.
- **Gaps & risks:** No hot-plug detection, retry queues, or telemetry for peripherals; device failures will block lanes without diagnostics.
- **Persona impact:** Cashiers cannot recover quickly from hardware glitches; IT lacks visibility into device health.
- **Acceptance coverage:** No automated coverage for device state transitions.

## 9. Observability & SLOs

- **Current footing:** Auth Service and Integration Gateway expose Prometheus counters for limited aspects (`services/auth-service/src/metrics.rs:5`, `services/integration-gateway/src/metrics.rs:5`). Other core services (Order, Payment, Inventory, Loyalty) do not publish metrics or health dashboards tailored to journey KPIs.
- **Gaps & risks:** No end-to-end latency tracking, Kafka lag monitoring, or store health dashboards; offline queue depth and tap counts are uninstrumented.
- **Persona impact:** IT and HQ cannot enforce the <2s/<5-tap SLO or respond proactively to outages.
- **Acceptance coverage:** No alerting or performance regression tests tied to KPIs.

## 10. Security, Privacy & Compliance

- **Current footing:** Services manually enforce tenant scoping via headers (`services/order-service/src/order_handlers.rs:34`, `services/inventory-service/src/inventory_handlers.rs:26`). Customer Service uses per-tenant DEKs derived from a master key for encrypted PII and GDPR helpers (`services/customer-service/src/main.rs:12`, `services/customer-service/src/main.rs:103`, `services/customer-service/src/main.rs:212`). Auth Service provides JWT verification and MFA flows but over HTTP only.
- **Gaps & risks:** No mTLS between services, no system-wide per-tenant key management, no immutable audit logging for sensitive actions beyond product edits, and no automated key rotation or DSR proofing across services.
- **Persona impact:** IT admins cannot demonstrate tenant isolation or compliance readiness; finance & auditors lack tamper-evident trails.
- **Acceptance coverage:** GDPR flows are concentrated in Customer Service; cross-service tests for tenant leakage or key rotation are absent.

## 11. Promotion & Price Governance

- **Current footing:** Only a front-end promo code field exists without backend enforcement (`frontends/pos-app/src/pages/EcommerceTemplate.tsx:26`).
- **Gaps & risks:** No promo engine, scheduling, approval workflow, or simulation tooling; markdowns cannot be controlled centrally.
- **Persona impact:** HQ cannot orchestrate promotions; stores cannot execute localized markdowns safely; shoppers may see inconsistent pricing.
- **Acceptance coverage:** None.

## 12. Data & Analytics Plumbing

- **Current footing:** Analytics Service aggregates `order.completed` and low-stock events into in-memory totals and emits basic alerts (`services/analytics-service/src/main.rs:27`, `services/analytics-service/src/main.rs:37`).
- **Gaps & risks:** No defined event contracts, CDC into a warehouse, anomaly detection, or forecasting baseline; stats reset on restart.
- **Persona impact:** HQ lacks reliable dashboards, and managers do not get actionable insights.
- **Acceptance coverage:** No tests for event ingestion accuracy or alert thresholds.

## 13. E-receipts, Communications & Post-purchase

- **Current footing:** The codebase has no notifications service; communication is limited to console logs (e.g., payment stubs) with no templating or multi-channel delivery.
- **Gaps & risks:** No e-receipts, pickup-ready notifications, or feedback loops; branding per tenant is absent.
- **Persona impact:** Shoppers miss digital confirmations; support cannot rely on consistent messaging.
- **Acceptance coverage:** None.

## 14. Tasking & Checklists

- **Current footing:** No task or checklist service exists.
- **Gaps & risks:** Store directives and open/close procedures remain manual; there is no POS/Admin integration for operational execution.
- **Persona impact:** Store managers cannot track completion or accountability.
- **Acceptance coverage:** None.

## 15. Identity & Access at Scale

- **Current footing:** Auth Service manages tenants, users, roles, MFA, and JWT issuance (`services/auth-service/src/user_handlers.rs:1010`), but roles are static and scoped to predefined strings.
- **Gaps & risks:** No SSO (SAML/OIDC) integration, no attribute-based overrides, and no automated provisioning pipeline.
- **Persona impact:** IT administrators must onboard manually and cannot enforce least privilege per store cluster; managers lack scoped self-service.
- **Acceptance coverage:** User provisioning tests focus on local flows; no coverage for external identity providers.

## Roadmap Alignment

- **Now (MVP hardening):** Idempotent checkout, payment orchestration, returns, loyalty, BOPIS, observability, and security all require foundational work�none are production-ready. The action plan�s "Now" slate aligns with the largest gaps identified above.
- **Next (v1.1+):** Variant catalog, mobile approvals, device abstraction, and promo governance currently have zero implementation; they naturally fall into the post-MVP bucket.
- **Explore / R&D:** Advanced analytics, tasking, and SSO/ABAC are absent today, validating their placement in the exploration phase.

## Cross-cutting Observations

- **Tenancy hygiene:** Each service re-implements `tenant_id_from_request`, increasing the chance of drift (`services/order-service/src/order_handlers.rs:34`, `services/inventory-service/src/inventory_handlers.rs:26`). A shared middleware should precede deeper tenant isolation.
- **API hygiene:** Public REST routes lack versioning and idempotency headers; webhook retry semantics are not defined in Integration Gateway (`services/integration-gateway/src/integration_handlers.rs:39`).
- **Edge conflict policy:** There is no documented resolution between price-lock promises and catalog changes; POS simply replays totals without validation (`frontends/pos-app/src/OrderContext.tsx:386`).
- **Saga coordination:** Multi-service workflows (returns, BOPIS, refunds) currently rely on fire-and-forget Kafka events without compensating actions (`services/integration-gateway/src/integration_handlers.rs:266`, `services/order-service/src/order_handlers.rs:210`).
- **Time zones & day boundaries:** Order timestamps default to `NOW()` UTC without store-local boundaries (`services/order-service/migrations/2002_extend_orders.sql:1`), risking reporting drift.
- **Performance budgets:** No service exposes latency histograms or tap counts to validate the 2-second / 5-tap goals.
- **Compliance runway:** PCI scope is partially addressed via payment stubs, but broader SOC2/GDPR evidence trails are missing outside Customer Service encryption helpers.
