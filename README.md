# NovaPOS

Cloud-native, multi-tenant, offline-capable Point of Sale platform.

## Money (Monetary Handling)

The `common-money` crate centralizes rounding logic (HalfUp default, Truncate, Bankers) and monetary utilities.

- Developer Guide: `docs/financial/money.md`
- Benchmarks Baseline: `docs/financial/benchmarks/money_rounding_baseline.md`
- Integer Cents RFC: `docs/rfcs/money_integer_cents.md`
- CHANGELOG: `CHANGELOG.md`

CI validates rounding behavior across all modes via `money-rounding-matrix` workflow.

## SQLx Offline Verification

This repo uses SQLx compile-time offline metadata. See `docs/development/sqlx-offline.md` for:

- How to regenerate per-query metadata (`.sqlx/` directories)
- Pruning stale query metadata
- Diff / drift detection in CI
- Converting runtime queries to macros

CI enforces `SQLX_OFFLINE=1` builds via the `sqlx-offline` workflow.

### Inventory Service Notes

For rationale on using dynamic `sqlx::query` in integration tests (to avoid maintaining offline macro metadata for ephemeral seed/setup SQL), see `services/inventory-service/README.md`.

## Layout

- services/ (Rust microservices)
- frontends/ (React apps)
- infra/ (Terraform, K8s manifests)
- local/ (docker-compose for Postgres, Redis, Kafka)

## Quick demos and scripts

See `scripts/README.md` for Windows-friendly try-it scripts:

- `try-compute.ps1` — header vs DB tax override demo
- `try-precedence.ps1` — location_id/pos_instance_id precedence demo
- `seed-skus-and-order.ps1` — seed SKUs, compute, optionally create order via `/orders/sku`
