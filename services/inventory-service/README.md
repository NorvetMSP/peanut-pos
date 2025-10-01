# Inventory Service

This service manages product inventory quantities, reservations, and related Kafka events (low_stock, reservation.expired, audit events).

## Integration Tests & sqlx

We intentionally avoid using `sqlx::query!` / `query_as!` macros inside long‑running or containerized integration tests (see `tests/`). CI sets `SQLX_OFFLINE=true` to speed builds and prevent network access during compilation. The compile‑time macros require a prepared offline cache (`sqlx-data.json`) covering every query shape. Maintaining that cache for ephemeral test setup SQL (INSERT seed rows, ad‑hoc SELECT aggregates) caused friction and frequent CI breaks.

Therefore the integration tests use runtime `sqlx::query` with `.bind(...)` to:

- Eliminate the need to regenerate offline metadata for seed queries.
- Allow schema evolution (new columns/tables) without blocking on `cargo sqlx prepare` before pushing drafts.
- Keep compile-time validation for core application code only (handlers & sweeper logic) where stability matters.

If you add new application queries (non-test), prefer macros for static validation. For test-only bootstrap SQL, prefer dynamic queries.

## Seeding Helpers

A reusable seeding helper is placed in `tests/test_utils.rs` to create a tenant, product, default location, legacy inventory row, and multi-location inventory item. Use it to reduce duplication across future integration tests.

## Metrics

Core metrics (counters + histograms) are extracted into the `common-observability` crate—see that crate for definitions.

## Running Locally

```bash
cargo run -p inventory-service
```

Environment variables of interest:

- `DATABASE_URL` – Postgres connection
- `KAFKA_BOOTSTRAP` – Kafka/Redpanda bootstrap servers
- `MULTI_LOCATION_ENABLED` – enable location-aware paths
- `RESERVATION_DEFAULT_TTL_SECS` / `RESERVATION_EXPIRY_SWEEP_SECS`
- `INVENTORY_DUAL_WRITE` – dual-write validation logging

