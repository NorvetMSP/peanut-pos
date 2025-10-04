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

## Helpful Scripts

- `./Makefile.ps1` — Start/Stop infra, build, test, run service, frontends
- `./migrate-all.ps1` — Apply migrations for all services
- `./regenerate-sqlx-data.ps1` — Generate per-query SQLx offline metadata (.sqlx/) with prune/reset options
- `scripts/run-order-local.ps1` — Run order-service locally with Windows-friendly feature flags and dev JWT key
- `scripts/seed-and-compute.ps1` — Seed SKUs (STD + EXEMPT) and POST /orders/compute with tax override header
- `scripts/create-order-with-payment.ps1` — Create a paid order (cash) and print the receipt with change due

## Deeper Dives

- Development bootstrap & tooling: development/dev-bootstrap.md
- SQLx offline verification & regeneration: development/sqlx-offline.md
- Tenancy + audit extraction background: development/audit-tenancy.md
- Security wave plan and follow-ups: security/wave5-follow-on-tickets.md
- Product-service audit view redaction: ../services/product-service/README.md

## Change Log & Roadmap

- Monetary handling changes and roadmap: ../CHANGELOG.md and financial/money.md

---

Maintainers: keep this runbook as the entry point. Add links to new service docs, update ports and scripts as they evolve, and ensure on-call runbooks under docs/security/ remain in sync with monitoring and alerting configs.
