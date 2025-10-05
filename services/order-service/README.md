# Order Service

A Rust/Axum microservice that handles order creation, compute, payments, refunds, and receipts for NovaPOS.

## Local development

- Rust toolchain is pinned via `rust-toolchain.toml` at the repo root.
- Dependencies: Postgres, optional Kafka (feature-gated).
- Common Windows note: prefer running tests in `--release` to avoid debug binary file-lock issues during rebuilds.

## Running

The service reads configuration from environment variables:

- DATABASE_URL: Postgres connection string
- JWT_ISSUER, JWT_AUDIENCE: JWT verification config
- Optional JWT_JWKS_URL: JWKS endpoint
- Optional JWT_DEV_PUBLIC_KEY_PEM: Dev-only RSA public key PEM for tests/local runs (kid="local-dev")
- INVENTORY_SERVICE_URL: Base URL for inventory-service (default <http://localhost:8087>)
- DEFAULT_TAX_RATE_BPS: Fallback tax rate in basis points (0 if unset)

Start the service (example):

```powershell
$env:DATABASE_URL = "postgres://postgres:postgres@localhost:5432/postgres"
$env:JWT_ISSUER = "https://auth.novapos.local"
$env:JWT_AUDIENCE = "novapos-admin"
cargo run -p order-service --release
```

## Tests

There are two kinds of tests:

- Unit tests and pure computations (no DB)
- Integration tests (feature-gated) that require Postgres and real JWT verification

To run the integration tests:

1) Provide a Postgres instance via TEST_DATABASE_URL. Tests will skip if not set or unreachable.

```powershell
$env:TEST_DATABASE_URL = "postgres://postgres:postgres@localhost:5432/postgres"
```

1) Run the tests in release mode:

```powershell
cargo test --manifest-path .\services\Cargo.toml -p order-service --no-default-features --features integration-tests --tests --release -- --test-threads=1
```

### Inventory bypass

The integration tests use an in-process Axum router and bypass inventory to keep flows deterministic and fast.

- Set `ORDER_BYPASS_INVENTORY=1` to skip calling the external inventory service.
- The test harness sets this automatically in-process.

### JWT for tests

Integration tests generate an ephemeral RSA key per run, set `JWT_DEV_PUBLIC_KEY_PEM`, and mint a token with `kid=local-dev`, `iss`=`JWT_ISSUER`, and `aud`=`JWT_AUDIENCE`.

To run local manual tests, you can also set `JWT_DEV_PUBLIC_KEY_PEM` to the contents of `jwt-dev.pub.pem` and mint tokens with `scripts\mint-dev-jwt.js`.

## Tax overrides

Admin endpoints allow setting tax rate overrides by scope:

- Tenant-wide
- Per location
- Per POS instance (wins over location)

Precedence: POS instance > Location > Tenant > DEFAULT_TAX_RATE_BPS/env/header.

The `/orders/compute` endpoint accepts optional `location_id` and `pos_instance_id` to drive tax resolution.

### Curl examples

Assuming you have a bearer token in $TOKEN and a tenant ID in $TENANT:

Set a tenant-level override at 8%:

```bash
curl -sS -X POST "$ORDER_URL/admin/tax_rate_overrides" \
  -H "Content-Type: application/json" \
  -H "X-Tenant-ID: $TENANT" \
  -H "X-Roles: admin" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"rate_bps":800}'
```

Compute with header override (5%) using product_id:

```bash
curl -sS -X POST "$ORDER_URL/orders/compute" \
  -H "Content-Type: application/json" \
  -H "X-Tenant-ID: $TENANT" \
  -H "X-Roles: cashier" \
  -H "Authorization: Bearer $TOKEN" \
  -H "x-tax-rate-bps: 500" \
  -d '{"items":[{"product_id":"UUID-HERE","quantity":1}]}'
```

## Windows-specific tips

- Use `--release` for tests to avoid transient file-lock issues.
- If you hit Postgres auth issues, confirm `TEST_DATABASE_URL` or `DATABASE_URL` are correct and reachable.
- Kafka is feature-gated; avoid enabling on Windows unless you have the dependencies.

## Try-it scripts

Quick PowerShell scripts to exercise typical flows from your Windows shell:

- scripts/try-compute.ps1: Seed one product and compute totals with a header override, then upsert a tenant DB override and compute again.
- scripts/try-precedence.ps1: Demonstrate tax override precedence end-to-end by creating tenant, location, and pos overrides and calling /orders/compute with location_id and pos_instance_id.
- scripts/seed-skus-and-order.ps1: Seed two products with SKUs and compute totals using SKUs; optionally create a real order via /orders/sku and print a plaintext receipt.

Each script can mint a dev JWT automatically using scripts/mint-dev-jwt.js (kid=local-dev) or accept a -Token you provide. Pass -TenantId to reuse an existing tenant; otherwise a GUID is generated.
