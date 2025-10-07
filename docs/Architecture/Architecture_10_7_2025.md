# NovaPOS Cloud POS Platform — Current Architecture Overview

NovaPOS is a distributed edge/cloud POS platform designed for multi-tenant SaaS. Stores run an offline-capable POS Edge Client, while cloud microservices (Rust, containerized) provide core business functions. Components are decoupled with events over Kafka and secured end-to-end for tenant isolation.

![Architecture overview](./architecture-overview.svg)

```mermaid
%%{init: { 'theme': 'neutral', 'themeVariables': { 'primaryColor': '#f7f9fc', 'lineColor': '#555', 'fontFamily': 'Inter, Segoe UI, Arial, sans-serif' } }}%%
flowchart LR
  %% Frontends
  subgraph Frontends
    POS["POS Edge Client\nReact PWA (offline)"]
    ADMIN["Admin Portal\nManagement UI"]
  end

  %% Kafka
  KAFKA["Kafka\norder.completed | order.voided | payment.completed"]

  %% Services
  subgraph Services (Rust)
    AUTH[Auth]
    PROD[Product]
    INV[Inventory]
    ORD[Order]
    PAY[Payment]
    IGW[Integration GW]
    CUST[Customer]
    LOY[Loyalty]
    ANA[Analytics]
  end

  %% Infra
  subgraph Infrastructure
    PG[("PostgreSQL\ntenant_id")]
    REDIS[(Redis)]
    VAULT[(Vault)]
  end

  %% Edges (REST)
  POS -- REST/JWT + X-Tenant-ID --> ORD
  ADMIN -- REST/JWT + X-Tenant-ID --> AUTH
  ADMIN -- REST --> PROD

  %% Sync between services
  ORD -- Reserve stock (REST) --> INV
  ORD -- Initiate payment (REST) --> PAY

  %% Events (Kafka)
  ORD -- order.completed --> KAFKA
  ORD -- order.voided --> KAFKA
  PAY -- payment.completed --> KAFKA
  KAFKA -- consume --> INV
  KAFKA -- consume --> LOY
  KAFKA -- consume --> ANA

  %% Data stores
  AUTH --- VAULT
  ORD --- PG
  PROD --- PG
  INV --- PG
  PAY --- VAULT
  IGW --- VAULT
  ORD --- REDIS
```

Legend: Solid arrows denote synchronous REST calls; arrows to/from Kafka nodes indicate event publication/consumption; dashed/labelled edges are illustrative only.

## System architecture overview

- Edge clients (React PWA) operate offline and sync when connectivity resumes.
- Cloud services (Rust) run in containers/orchestrators (Docker/Kubernetes).
- Asynchronous events on Kafka decouple workflows; REST handles synchronous calls.
- Strict multi-tenancy with tenant-scoped data and request validation.

When an order is voided, the Order Service emits an `order.voided` event for other services to consume instead of making direct calls. Similar topics like `order.completed` and `payment.completed` propagate state changes without tight coupling.

Requests must include an `X-Tenant-ID` header that matches the user’s JWT claims. All records carry a tenant identifier to ensure isolation.

## Edge POS and offline mode

The POS Edge Client is a React web app (tablet/kiosk friendly) deployed per store. It supports scanning, carts, payments, and receipt printing with a touch-first UX.

### Key capabilities

- Offline-first caching of catalog, pricing, and tax rules (IndexedDB/localStorage).
- Local transaction queue with idempotency keys; syncs to Order Service when online.
- Deferred updates: catalog/price changes apply on reconnection.
- Secure transport: TLS everywhere and device authentication.

Note: While syncing, the UI may show cached receipt data until cloud confirmation, leading to small, later-reconciled differences.

## Cloud backend and microservices

Containerized Rust services are deployable to Kubernetes or Docker Compose.

### Core services

- [Auth Service](../../services/auth-service/)
- [Product Service](../../services/product-service/)
- [Inventory Service](../../services/inventory-service/)
- [Order Service](../../services/order-service/)
- [Payment Service](../../services/payment-service/)
- [Integration Gateway](../../services/integration-gateway/)
- [Customer Service](../../services/customer-service/)
- [Loyalty Service](../../services/loyalty-service/)
- [Analytics Service](../../services/analytics-service/)

### Architecture highlights

- REST APIs today; GraphQL planned.
- Kafka for async events; REST for synchronous paths (e.g., Order → Inventory reservation).
- Multi-tenant PostgreSQL with `tenant_id` for row-level isolation.
- Shared infra: PostgreSQL, Redis, Kafka, Zookeeper, Vault.
- IaC with Terraform for AWS deployments.
- Independent deployability with health endpoints and observability.

## Multi-tenancy and data isolation

NovaPOS is built as a true multi-tenant SaaS with isolation at multiple layers.

### Enforcement layers

- Database: `tenant_id` on every table, tenant-scoped indexes.
- Application: `X-Tenant-ID` header + JWT claim validation via `SecurityCtxExtractor`.
- RBAC: roles (cashier, manager, admin) control UI and API privileges.
- Encryption: tenant-specific keys (envelope encryption via Vault).

This ensures both logical isolation (tenant-scoped queries) and cryptographic protection (per-tenant encryption).

## Secure integration and omnichannel

The Integration Gateway acts as a secure façade for partner APIs and omnichannel flows.

### Capabilities

- Handles inbound/outbound APIs and webhooks (e-commerce, payment gateways).
- API keys or OAuth tokens, rate limiting (Redis), and HMAC signature checks.
- HTTPS everywhere; all integration traffic is audited.

Typical scenarios:

- Online orders → in-store inventory sync.
- Daily sales → accounting systems.
- Loyalty data → marketing platforms.

## AI and analytics stack

The Analytics & AI Service (port `8082`) consumes transactional and inventory events to power insights.

### Current functionality

- Aggregations: sales summaries, top sellers.
- Early AI: anomaly detection (e.g., refund spikes) and trend forecasting.
- Roadmap: demand forecasting, stock optimization, ML model integration.

Architecture is ready to offload to an analytical store or warehouse (e.g., BigQuery). Data is isolated per tenant and can be anonymized for cross-tenant learning.

## Components

### 1. POS Edge Client (frontend)

- React PWA optimized for touchscreen.
- Works offline (IndexedDB + local queue).
- Uses JWT + `X-Tenant-ID` for secure API calls.
- Prints receipts; payment terminal integration planned.
- Idempotency prevents duplicate orders during resync.

### 2. Back-office Admin Portal

- React web app (port `3001`) for management tasks.
- Features:
  - Product and inventory CRUD.
  - User management with role enforcement.
  - Order/return workflows.
  - Integration settings (API keys, tax rules).
  - Dashboards with analytics and alerts.
- Auth: JWT; routes guarded by role (e.g., `RequireRoles(["admin"])`).

### 3. Authentication and User Service (Auth Service)

- Port `8085`.
- Manages users, tenants, roles.
- Issues JWTs (RS256) with claims: `sub`, `tid`, `roles`.
- MFA (TOTP) for managers/admins.
- JWKS endpoint for token validation.
- API keys for external integrations.
- Comprehensive auth logging and audit.

### 4. Product and Inventory Services

- Product Service (port `8081`): catalog CRUD, audit logs, price/tax data.
- Inventory Service (port `8087`): stock tracking, reservations, low-stock alerts.
- Shared money library for consistent rounding.
- Emits events like `product.updated`, `inventory.low_stock`.
- Supports multi-location inventory and a reservation TTL sweeper.

### 5. Order and Transaction Service

- Port `8084`.
- Manages orders, refunds, and voids.
- Responsibilities:
  - Validate and compute order totals.
  - Reserve inventory.
  - Initiate payments.
  - Generate receipts.
- Enforces idempotency to prevent duplicates.
- Consumes/publishes payment status via Kafka (e.g., `payment.completed`).
- Emits events for Inventory and Loyalty.
- Supports refunds/voids; automation via integrations is planned.

### 6. Payment Services

- Payment Service (port `8086`): handles card transactions (currently stubbed).
- Integration Gateway (port `8083`): manages crypto via Coinbase Commerce.

Card payments:

- Placeholder for Valor PayTech terminal integration.
- Simulated approval workflow for MVP (no PAN/CVV stored).
- Future support: EMV, tap, signature capture.

Crypto payments:

- Integrates with Coinbase Commerce.
- Creates charge, receives webhook, publishes `payment.completed` to Kafka.
- Supports USDC and confirms blockchain transactions.
- Secrets in Vault; minimal PCI scope.

### 7. Customer and Loyalty Services

- Customer Service (port `8089`):
  - Stores encrypted PII (per-tenant keys).
  - CRUD and GDPR endpoints.
- Loyalty Service (port `8088`):
  - Tracks points earned/redeemed.
  - Consumes `order.completed` events.
  - Manual adjustments and redemption API (in progress).
- Async by design: sales complete even if Loyalty is temporarily offline.
- Planned: tiering, offline caching, CRM integration.

### 8. Reporting and audit logging

- Common audit library produces Kafka `audit.events`.
- Append-only audit log planned for centralized persistence.
- Operational reporting via Order/Analytics APIs.
- Monitoring:
  - Prometheus/Grafana dashboards.
  - Structured JSON logs.
  - `trace_id` correlation across services.
- Planned: dedicated reporting + immutable audit microservice.

## Data flows

### Checkout (card)

1. POS Edge creates order draft locally; on submit, sends to Order Service (REST, JWT + `X-Tenant-ID`).
2. Order Service validates, computes totals, reserves inventory (sync call to Inventory).
3. Order Service initiates payment via Payment Service.
4. Payment Service authorizes (stub/terminal); publishes `payment.completed` (Kafka).
5. Order Service marks order complete and publishes `order.completed` (Kafka).
6. Inventory consumes `order.completed` and finalizes stock changes; Loyalty accrues points.

```mermaid
%%{init: { 'theme': 'neutral' }}%%
sequenceDiagram
  autonumber
  participant POS as POS Edge
  participant ORD as Order Service
  participant INV as Inventory
  participant PAY as Payment Service
  participant K as Kafka
  participant LOY as Loyalty

  POS->>ORD: Create/submit order (REST, JWT + X-Tenant-ID)
  ORD->>INV: Reserve stock (REST)
  INV-->>ORD: Reservation OK
  ORD->>PAY: Initiate payment (REST)
  PAY-->>K: payment.completed
  K-->>ORD: payment.completed
  ORD-->>K: order.completed
  K-->>INV: order.completed
  K-->>LOY: order.completed
```

Legend: Solid arrows show requests; `-->>` indicates messages/events; K represents Kafka topics; step numbers are for readability.

### Refund

1. Admin Portal requests refund to Order Service (REST).
2. Order Service validates policy and publishes `order.voided` or `order.refunded` (Kafka).
3. Payment Service processes reversal; emits `payment.completed` (Kafka) with refund metadata.
4. Inventory and Analytics consume events for stock and reporting updates.

```mermaid
%%{init: { 'theme': 'neutral' }}%%
sequenceDiagram
  autonumber
  participant ADMIN as Admin Portal
  participant ORD as Order Service
  participant PAY as Payment Service
  participant K as Kafka
  participant INV as Inventory
  participant ANA as Analytics

  ADMIN->>ORD: Request refund (REST)
  ORD-->>K: order.refunded / order.voided
  ORD->>PAY: Initiate reversal (REST)
  PAY-->>K: payment.completed (refund)
  K-->>INV: order.refunded / order.voided
  K-->>ANA: order.refunded / payment.completed
```

Legend: Admin initiates via REST; Order emits `order.refunded`/`order.voided`; Payment emits `payment.completed (refund)`; Inventory/Analytics consume events.

### Crypto payment (Coinbase Commerce)

1. POS Edge selects crypto; Order Service asks Integration Gateway to create a charge.
2. Shopper completes payment; Coinbase sends webhook to Integration Gateway.
3. Integration Gateway verifies HMAC, then publishes `payment.completed` (Kafka).
4. Order Service consumes `payment.completed`, completes the order, emits `order.completed`.

```mermaid
%%{init: { 'theme': 'neutral' }}%%
sequenceDiagram
  autonumber
  participant POS as POS Edge
  participant ORD as Order Service
  participant IGW as Integration Gateway
  participant CB as Coinbase Commerce
  participant K as Kafka

  POS->>ORD: Start crypto checkout (REST)
  ORD->>IGW: Create charge (REST)
  IGW->>CB: Create charge (API)
  CB-->>IGW: Webhook (payment confirmed)
  IGW->>IGW: Verify HMAC signature
  IGW-->>K: payment.completed (crypto)
  K-->>ORD: payment.completed
  ORD-->>K: order.completed
```

## Entity relationships by bounded context

### Order context

```mermaid
%%{init: { 'theme': 'neutral' }}%%
classDiagram
  class Tenant {
    +uuid id
    +string name
  }
  class Order {
    +uuid id
    +uuid tenant_id
    +uuid customer_id
    +Money total
    +OrderStatus status
  }
  class OrderLine {
    +uuid id
    +uuid order_id
    +uuid product_id
    +int qty
    +Money unit_price
  }
  class Payment {
    +uuid id
    +uuid order_id
    +string method
    +Money amount
    +PaymentStatus status
  }
  Tenant <|-- Order
  Order "1" -- "*" OrderLine
  Order "1" -- "*" Payment
  OrderLine ..> Product : product_id
```

Legend: `<|--` indicates tenant scoping; `1--*` shows cardinality; dotted `..>` shows a reference to an external context (Product).

### Product context

```mermaid
%%{init: { 'theme': 'neutral' }}%%
classDiagram
  class Tenant {
    +uuid id
    +string name
  }
  class Product {
    +uuid id
    +uuid tenant_id
    +string name
    +Money price
  }
  class InventoryItem {
    +uuid id
    +uuid tenant_id
    +uuid product_id
    +int on_hand
    +int reserved
    +string location_id
  }
  Tenant <|-- Product
  Tenant <|-- InventoryItem
  Product "1" -- "*" InventoryItem
```

Legend: Inventory is per-product and per-location; fields simplified (pricing/tax rules omitted for clarity).

### Customer and Loyalty context

```mermaid
%%{init: { 'theme': 'neutral' }}%%
classDiagram
  class Tenant {
    +uuid id
    +string name
  }
  class Customer {
    +uuid id
    +uuid tenant_id
    +PII encrypted_fields
  }
  class LoyaltyAccount {
    +uuid id
    +uuid customer_id
    +int points
  }
  Tenant <|-- Customer
  Customer "1" -- "0..1" LoyaltyAccount
```

Legend: Customer PII is encrypted; loyalty account is optional and belongs to a single customer.

## Conclusion

NovaPOS delivers a cloud-native, multi-tenant POS with offline resilience, modular microservices, and an event-driven core.

### Strengths

- Modular and decoupled via Kafka events.
- Offline-capable edge clients.
- Strong tenant isolation and encryption.
- Extensible for omnichannel and AI integrations.

### Evolving areas

- Hardware payment terminal integration.
- Automated refunds and reversals.
- Advanced analytics and loyalty tiering.
- Dedicated audit/reporting service.

## Sources

Derived from the current NovaPOS codebase, architecture proposals, MVP runbooks, and configuration files.
