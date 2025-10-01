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

## Layout

- services/ (Rust microservices)
- frontends/ (React apps)
- infra/ (Terraform, K8s manifests)
- local/ (docker-compose for Postgres, Redis, Kafka)
