# NovaPOS Runbook

A concise, linked guide for developers and on-call to bring up the stack, run services, manage DB + SQLx metadata, test, monitor, and handle security workflows.

---

## Quick Start (First Time)

- Read: development/dev-bootstrap.md for the canonical bootstrap flow on Windows/PowerShell (also applicable to macOS/Linux with minor changes).
- TL;DR PowerShell (local dev):

  ```powershell
  ./Makefile.ps1 Start-Infra
  ./migrate-all.ps1
  ./regenerate-sqlx-data.ps1 -Prune
  ./Makefile.ps1 Run-Service -Name auth-service
  ```

- Full stack (everything via Docker):

  ```powershell
  docker compose up --build -d
  docker compose up kafka-topics-init
  ```

### Order-service local run (Windows-friendly)

For fast iteration on order totals, receipts, and POS flows without native Kafka linking on Windows:

```powershell
# Starts dockerized infra (Postgres/Kafka/ZK) if you haven't already
./Makefile.ps1 Start-Infra

# Run order-service locally with Windows-safe features and dev JWT key
scripts/run-order-local.ps1
```

What the script does:

- Sets DATABASE_URL to the dockerized Postgres (localhost:5432/novapos)
- Sets JWT_ISSUER and JWT_AUDIENCE
- Loads jwt-dev.pub.pem into JWT_DEV_PUBLIC_KEY_PEM to verify tokens without JWKS
- Runs order-service with the kafka-core feature to avoid native rdkafka linkage

## Daily Dev Loop

- Infra only (fast inner loop): `./Makefile.ps1 Start-Infra`
- Build all: `./Makefile.ps1 Build-Services`
- Run one service natively: `./Makefile.ps1 Run-Service -Name <service>`
- Run tests (unit): `pushd services; cargo test --workspace; popd`
- Run integration tests: see tests/integration.md and enable with `--features integration`.
- Update SQLx offline metadata after changing macro queries: `./regenerate-sqlx-data.ps1 -Prune`

### Live sanity check for /orders/compute

We added a reusable compute path and a small script to seed two SKUs and call the endpoint.

```powershell
# Ensure order-service is running (either locally or in Docker). If Docker is running order-service, it owns 8084.

# Seed two products (Soda: STD, Water: EXEMPT) and call POST /orders/compute
powershell -File scripts/seed-and-compute.ps1

# Optional: set a fixed tenant for repeatable calls
$env:SEED_TENANT_ID = [guid]::NewGuid().ToString()
powershell -File scripts/seed-and-compute.ps1
```

### Create an order with payment (cash)

We added a helper script to create a PAID order using cash and show the receipt with change due.

```powershell
# Ensure order-service is running and Postgres is available
powershell -File scripts/run-payment-demo.ps1  # mints JWT + runs cash demo end-to-end
# Or run the script directly if you already have AUTH_BEARER set
powershell -File scripts/create-order-with-payment.ps1
```

What the script does:

- Seeds Soda (STD) and Water (EXEMPT) under a tenant (uses SEED_TENANT_ID if set)
- Computes totals via /orders/compute
- Posts /orders/sku with a cash payment that exceeds the total to demonstrate change
- Fetches the plaintext receipt; look for Paid (cash) and Change lines

Expected JSON:

```json
{
  "items": [
    { "sku": "SKU-SODA", "name": "Soda Can", "qty": 2, "unit_price_cents": 199, "line_subtotal_cents": 398, "tax_code": "STD" },
    { "sku": "SKU-WATER", "name": "Bottle Water", "qty": 1, "unit_price_cents": 149, "line_subtotal_cents": 149, "tax_code": "EXEMPT" }
  ],
  "subtotal_cents": 547,
  "discount_cents": 55,
  "tax_cents": 29,
  "total_cents": 521
}
```

Notes:

- Tax rate precedence for compute:
  1) Request body `tax_rate_bps` (explicit override)
  2) Headers `X-Tax-Rate-Bps` or `X-Tenant-Tax-Rate-Bps`
  3) DB overrides in `tax_rate_overrides` table, with scope precedence: POS instance > Location > Tenant
  4) Env default `DEFAULT_TAX_RATE_BPS`
- Taxability uses `products.tax_code` (EXEMPT/ZERO/NONE → non-taxable; missing/STD → taxable).

### Create an order with payment (card)

To simulate a card authorization and capture (mocked):

```powershell
# Requires a valid JWT in AUTH_BEARER or use run-payment-demo to mint one first
powershell -File scripts/create-order-with-card.ps1
```

Result: order is marked paid with method=card; receipt shows a Paid line without Change.

### Payment intents (MVP)

We added a basic payment intent lifecycle in payment-service with optional DB. Order-service can initiate an intent during card checkout when enabled.

- Toggle: set `ENABLE_PAYMENT_INTENTS=1` in order-service to POST a create-intent request to payment-service. Configure `PAYMENT_SERVICE_URL` (default <http://localhost:8086>).
- Endpoints (payment-service):
  - POST `/payment_intents` { id, orderId, amountMinor, currency, idempotencyKey? } → { id, state }
  - GET `/payment_intents/:id` → { id, state }
  - POST `/payment_intents/confirm` { id } → transitions created→authorized
  - POST `/payment_intents/capture` { id } → transitions authorized→captured
  - POST `/payment_intents/void` { id } → transitions authorized→voided
  - POST `/payment_intents/refund` { id } → transitions captured→refunded

Behavior:

- Without `DATABASE_URL`, handlers return stub states for local workflows (e.g., created/authorized) and do not persist.
- With `DATABASE_URL`, state transitions are enforced. Invalid transitions return HTTP 409 with code `invalid_state_transition`.
- Idempotency: unique constraint on `idempotency_key` when provided.

Refund/void passthrough (P6-03)

- When `DATABASE_URL` is set and an intent has `provider` and `provider_ref`, the payment-service will call a gateway abstraction during refund/void and persist any updated `provider_ref` returned by the gateway. In stub mode, the provider_ref is deterministically updated: `...-refund` for refunds and `...-void` for voids.
- Without DB, refund/void endpoints return stub states and do not call the gateway.
- Order-service wiring to call payment-service for refunds/voids will be guarded by a feature flag. Default `PAYMENT_SERVICE_URL` is `http://localhost:8086`.

### Webhook verification

Incoming webhooks are protected by an HMAC signature with timestamp skew and nonce replay checks. Enforcement is applied by middleware to any route under the path prefix `/webhooks/`.

- Required headers:
  - X-Signature: `sha256=<hex>` HMAC over the canonical string below
  - X-Timestamp: Unix epoch seconds
  - X-Nonce: Unique, single-use nonce per delivery
  - X-Provider: Optional provider tag, stored with the nonce for audit

- Canonical string to sign:
  - `ts:<X-Timestamp>\nnonce:<X-Nonce>\nbody_sha256:<sha256(body)>`

- Environment configuration:
  - WEBHOOK_ACTIVE_SECRET: HMAC secret used to validate signatures
  - WEBHOOK_MAX_SKEW_SECS: Max allowed clock skew in seconds (default 300)

- Behavior:
  - Missing or mismatched signature → 401 with `X-Error-Code: sig_missing|sig_mismatch`
  - Invalid timestamp or excessive skew → 401 with `X-Error-Code: sig_ts_invalid|sig_skew`
  - Nonce replay (seen before) → 401 with `X-Error-Code: sig_replay`
  - When no database is configured, signature and timestamp checks still apply; nonce replay protection is skipped.

- Storage:
  - Nonces are persisted in `webhook_nonces (nonce TEXT PRIMARY KEY, ts TIMESTAMPTZ DEFAULT now(), provider TEXT)`

Notes:

- The middleware is enabled; add webhook routes under `/webhooks/` to activate protection on those endpoints.

### DB-backed tax rate overrides

We support per-tenant, per-location, and per-POS-instance tax rate overrides. The resolver applies precedence:

- POS instance (most specific)
- Location
- Tenant (least specific)

When calling `/orders/compute`, you may pass context in the body:

```json
{
  "items": [{ "sku": "SKU-123", "quantity": 2 }],
  "discount_percent_bp": 500,
  "location_id": "<uuid>",
  "pos_instance_id": "<uuid>"
}
```

Admin API (tenant-scoped; roles: Admin/Manager):

- GET `/admin/tax_rate_overrides` — List overrides, with optional filters `?location_id=<uuid>&pos_instance_id=<uuid>`
- POST `/admin/tax_rate_overrides` — Upsert an override for the given scope

Example (PowerShell):

```powershell
$headers = @{ 'Authorization' = 'Bearer <dev-token>'; 'X-Tenant-ID' = '<tenant-uuid>'; 'X-Roles' = 'admin' }
Invoke-RestMethod -Method Post -Uri http://localhost:8084/admin/tax_rate_overrides -Headers $headers -ContentType 'application/json' -Body (@{
  location_id = $null
  pos_instance_id = $null
  rate_bps = 800
} | ConvertTo-Json)
```

Schema (order-service migration `2007_create_tax_rate_overrides.sql`):

```sql
CREATE TABLE IF NOT EXISTS tax_rate_overrides (
  tenant_id UUID NOT NULL,
  location_id UUID NULL,
  pos_instance_id UUID NULL,
  rate_bps INTEGER NOT NULL CHECK (rate_bps >= 0 AND rate_bps <= 10000),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

Troubleshooting:

- If you get 401/403, ensure `X-Tenant-ID` and `X-Roles: admin` or `manager` are present and your JWT is valid.
- If table is missing in dev, run `./migrate-all.ps1`. The POST handler also creates the table opportunistically for local testing.

## Services & Ports (default Compose)

- auth-service: <http://localhost:8085>
- integration-gateway: <http://localhost:8083>
- product-service: <http://localhost:8081>
- order-service: <http://localhost:8084>
- payment-service: <http://localhost:8086>
- inventory-service: <http://localhost:8087>
- loyalty-service: <http://localhost:8088>
- customer-service: <http://localhost:8089>
- Kafka UI: <http://localhost:8080>
- Prometheus: <http://localhost:9090>
- Grafana: <http://localhost:3002> (admin/admin)

All services expose `/healthz` and most expose `/metrics` (Prometheus format) once started.

## Databases & Migrations

- Shared dev DSN (default): `postgres://novapos:novapos@localhost:5432/novapos`
- Run all service migrations: `./migrate-all.ps1`
- Reset database + regenerate SQLx (dev only): `./regenerate-sqlx-data.ps1 -Prune -ResetDatabase`

See development/sqlx-offline.md for per-query SQLx metadata workflow, pruning, and CI patterns.

## Testing

- Workspace tests: `pushd services; cargo test --workspace; popd`
- Integration tests (opt-in):
  - Compute path integration is feature-gated inside order-service; enable with:

    ```powershell
    pushd services
    cargo test -p order-service --no-default-features --features "kafka-core integration-tests" --tests
    popd
    ```

  - Other workspace integration tests: `cargo test --manifest-path services/Cargo.toml --features integration`

- Auth embedded Postgres/test flags and examples: tests/README.md and tests/integration.md

Useful docs:

- tests/README.md — suite overview and commands
- tests/integration.md — feature flag, infra expectations, examples

### Frontend tests (Vitest & Playwright)

- POS App unit tests (frontends/pos-app):

  ```powershell
  pushd frontends/pos-app
  npm ci
  npm test
  popd
  ```

- Admin Portal unit tests (frontends/admin-portal):
  - Default (watch/dev UI):

    ```powershell
    pushd frontends/admin-portal
    npm ci
    npm run test
    popd
    ```

  - One-off run (no watch):

    ```powershell
    pushd frontends/admin-portal
    npx vitest run
    popd
    ```

- Admin Portal end-to-end (Playwright):
  - First-time only (install browsers):

    ```powershell
    pushd frontends/admin-portal
    npx playwright install
    popd
    ```

  - Headless run:

    ```powershell
    pushd frontends/admin-portal
    npm run test:e2e
    popd
    ```

  - Headed run (opens browser):

    ```powershell
    pushd frontends/admin-portal
    npm run test:e2e:headed
    popd
    ```

## Observability & Monitoring

- Bring up Prometheus + Grafana locally: `docker compose up -d prometheus grafana`
- Default Prometheus config: ../monitoring/prometheus/prometheus.yml
- Grafana provisioning: ../monitoring/grafana/provisioning/* (datasource + dashboards)
- Security dashboard and alert rules: security/prometheus-grafana-bootstrap.md

### POS telemetry smoke test (print retries and queue depth)

Use this to verify the POS → order-service telemetry ingestion and Prometheus metrics exposure.

Prereqs:

- order-service running and reachable on <http://localhost:8084>
- A dev JWT (see scripts/mint-dev-jwt.js or run-payment-demo.ps1) and a tenant UUID

PowerShell (POST a sample snapshot):

```powershell
$headers = @{ 'Content-Type' = 'application/json'; 'X-Tenant-ID' = '<tenant-uuid>'; 'Authorization' = 'Bearer <dev-jwt>' }
$body = @{
  ts = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  labels = @{ tenant_id = '<tenant-uuid>'; store_id = 'store-001' }
  counters = @(
    @{ name = 'pos.print.retry.queued';  value = 3 }
    @{ name = 'pos.print.retry.failed';  value = 1 }
    @{ name = 'pos.print.retry.success'; value = 0 }
  )
  gauges = @(
    @{ name = 'pos.print.queue_depth';        value = 4 }
    @{ name = 'pos.print.retry.last_attempt'; value = 1200 }
  )
} | ConvertTo-Json -Depth 6
Invoke-RestMethod -Method Post -Uri http://localhost:8084/pos/telemetry -Headers $headers -Body $body | Format-List
```

Verify metrics are exposed:

```powershell
curl http://localhost:8084/metrics | Select-String -Pattern 'pos_print_retry_total|pos_print_gauge'
```

Expected to see lines like:

```text
pos_print_retry_total{tenant_id="<tenant-uuid>",store_id="store-001",kind="queued"} 3
pos_print_retry_total{tenant_id="<tenant-uuid>",store_id="store-001",kind="failed"} 1
pos_print_gauge{tenant_id="<tenant-uuid>",store_id="store-001",name="queue_depth"} 4
```

Optional: check Prometheus UI at <http://localhost:9090> for the expressions used by alerts, e.g.:

```promql
increase(pos_print_retry_total{kind="failed"}[10m])
max_over_time(pos_print_gauge{name="queue_depth"}[5m])
```

### Alerts overview for POS print

At-a-glance rules, thresholds, and routing. Full rules live in `monitoring/prometheus/alerts/pos-print-telemetry.rules.yml`; routing in `monitoring/alertmanager/alertmanager.yml`.

- PosPrintQueueDepthHigh
  - Condition: `max_over_time(pos_print_gauge{name="queue_depth"}[5m]) >= 3`
  - Duration: 5m
  - Severity: warning → routed to Slack receiver

- PosPrintQueueDepthCritical
  - Condition: `max_over_time(pos_print_gauge{name="queue_depth"}[10m]) >= 10`
  - Duration: 10m
  - Severity: critical → routed to PagerDuty receiver

- PosPrintRetriesSpiking
  - Condition: `increase(pos_print_retry_total{kind="failed"}[10m]) >= 5`
  - Duration: immediate (no for:)
  - Severity: warning → routed to Slack receiver

- PosPrintNoSuccessAfterQueued
  - Condition: `increase(queued[15m]) > 0 and increase(success[15m]) == 0`
    - Concrete: `(increase(pos_print_retry_total{kind="queued"}[15m]) > 0) and (increase(pos_print_retry_total{kind="success"}[15m]) == 0)`
  - Duration: immediate (no for:)
  - Severity: critical → routed to PagerDuty receiver

Routing summary (Alertmanager):

- Grouping: `alertname, tenant_id, store_id`
- severity=warning → Slack (`receivers.slack`) via `${SLACK_WEBHOOK_URL}` to `#novapos-alerts`
- severity=critical → PagerDuty (`receivers.pagerduty`) via `${PAGERDUTY_ROUTING_KEY}`
- Default receiver: `dev-null` (no-op) if no child route matches

Tip: Set the required environment variables for receivers in your environment before bringing up Alertmanager, otherwise alerts will route but not deliver.

## Security & Environment Promotion

- Runbooks (auth telemetry, rate limit checks, PagerDuty wiring): security/README.md
- Secret promotion to staging/production: security/secret-promotion-guide.md
- End-to-end environment rollout notes: security/EnviromentPromotion.md
- Hardening notes for Rust services and checklists: security/security-hardening-rust-addendum.md

## Money (Rounding) Configuration

- Reference: financial/money.md and rfcs/money_integer_cents.md
- Runtime env var: `MONEY_ROUNDING` supports HalfUp (default), Truncate, Bankers. See CHANGELOG.md for recent behavior changes.

## Windows + Kafka (rdkafka) Notes

- If you hit unresolved zlib symbols during build on Windows/MSVC, follow development/windows-kafka-build.md for the working link-search + `-l zlib` solution and troubleshooting.

## Common Environment Variables (dev examples)

- `DATABASE_URL` — Postgres DSN (dev default above)
- `KAFKA_BOOTSTRAP` — `localhost:9092` (minimal) or `kafka:9092` (compose)
- `REDIS_URL` — `redis://localhost:6379/0`
- `JWT_ISSUER` — `https://auth.novapos.local`
- `JWT_AUDIENCE` — `novapos-frontend,novapos-admin,novapos-postgres`
- `MONEY_ROUNDING` — rounding mode for common-money
- Service-specific variables are documented in each service and compose file.

## Troubleshooting Cheatsheet

- SQLx offline missing files: re-run `./regenerate-sqlx-data.ps1 -Prune` (see development/sqlx-offline.md)
- Migration checksum mismatch: `./regenerate-sqlx-data.ps1 -Prune -ResetDatabase` (dev only)
- Integration tests skip: add `--features integration` and ensure infra is up (`docker compose up -d postgres kafka redis`)
- 405 on /orders/compute: Often means Docker’s order-service is still bound to 8084 while you’re hitting a local binary (or vice versa). Run `docker ps` to check, or stop the container:

  ```powershell
  docker compose stop order-service
  ```

- 401/403 on order endpoints: Include headers `X-Tenant-ID` and `X-Roles` with a role that passes checks (e.g., `admin`). When running natively, `JWT_DEV_PUBLIC_KEY_PEM` is accepted; in Docker, JWKS is used by default from auth-service.
- Windows Kafka link errors: development/windows-kafka-build.md
- Metrics not visible: bring up Prometheus/Grafana and confirm scrape targets; see security/prometheus-grafana-bootstrap.md

### Demo 3: end-to-end with inventory on

This demo seeds SKUs, computes totals, creates an order, reserves inventory, and prints a receipt. Inventory is enforced (no bypass). Requirements:

- JWT must include tid and a UUID sub; audience should be a single string that matches service config.
- Required headers for requests you issue: `Authorization: Bearer <token>`, `X-Tenant-ID: <uuid>`, and `X-Roles: admin,cashier`.
- The order-service forwards roles `Admin,Manager,Cashier` to inventory-service for server-to-server authorization.

How to run (PowerShell):

- Use `scripts/run-payment-demo.ps1` to mint a dev JWT and run the cash demo end-to-end; or use `scripts/seed-skus-and-order.ps1` directly if you already have a token.
- If `/orders/sku` returns `product_not_found`, the script automatically falls back to `/orders` using seeded product_ids and the cents returned by `/orders/compute` to ensure correct unit prices and totals.
- The compute call also has a fallback path: if SKU-based compute fails due to DB misalignment, it retries using the seeded product_ids.

Preflight DB alignment check:

- The smoke check and demo scripts perform a quick alignment check using a minted JWT with proper roles to avoid SKU mismatches across services.
- If misalignment is detected, re-run `./migrate-all.ps1` and reseed via the scripts.

Common issues:

- 403 from inventory: ensure your client `X-Roles` includes `manager` or use the helper scripts which set roles and the JWT properly; order-service will forward the required role set to inventory.
- Totals are $0.00 on receipt: ensure the order creation path used the compute response cents for `unit_price_cents`, `line_total_cents`, and `total_cents`. The provided scripts do this automatically.

## Helpful Scripts

- `./Makefile.ps1` — Start/Stop infra, build, test, run service, frontends
- `./migrate-all.ps1` — Apply migrations for all services
- `./regenerate-sqlx-data.ps1` — Generate per-query SQLx offline metadata (.sqlx/) with prune/reset options
- `scripts/run-order-local.ps1` — Run order-service locally with Windows-friendly feature flags and dev JWT key
- `scripts/seed-and-compute.ps1` — Seed SKUs (STD + EXEMPT) and POST /orders/compute with tax override header
- `scripts/create-order-with-payment.ps1` — Create a paid order (cash) and print the receipt with change due
- `scripts/create-order-with-card.ps1` — Create a paid order (card) and print the receipt
- `scripts/run-payment-demo.ps1` — Mint a dev JWT and run the cash payment demo end-to-end

## Deeper Dives

- Development bootstrap & tooling: development/dev-bootstrap.md
- SQLx offline verification & regeneration: development/sqlx-offline.md
- Tenancy + audit extraction background: development/audit-tenancy.md
- Security wave plan and follow-ups: security/wave5-follow-on-tickets.md
- Product-service audit view redaction: ../services/product-service/README.md

## Change Log & Roadmap

- Monetary handling changes and roadmap: ../CHANGELOG.md and financial/money.md
- MVP execution plan: RoadMap_Execution_Checklist.md

---

Maintainers: keep this runbook as the entry point. Add links to new service docs, update ports and scripts as they evolve, and ensure on-call runbooks under docs/security/ remain in sync with monitoring and alerting configs.
