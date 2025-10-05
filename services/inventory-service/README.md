# Inventory Service

This service manages product inventory quantities, reservations, and related Kafka events (low_stock, reservation.expired, audit events).

Low stock events

- Emitted to topic `inventory.low_stock` only when inventory crosses from above threshold to at-or-below threshold.
- Crossing rule: prev_quantity > threshold AND new_quantity <= threshold.
- This prevents repeated alerts while the item remains below threshold; alerts resume after stock is replenished above threshold and later crosses again.
- Threshold source: `inventory.threshold` (legacy) or `inventory_items.threshold` (multi-location; we use MIN across locations when aggregating).

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

## Windows tips: SQLx offline metadata

Regenerate SQLx offline metadata and reapply migrations using the script at the repo root. From PowerShell:

```powershell
# From the repository root
Set-Location -Path 'C:\Projects\novapos'
./regenerate-sqlx-data.ps1 -AutoResetOnChecksum

# Or from the services folder reference the parent path
Set-Location -Path 'C:\Projects\novapos\services'
..\regenerate-sqlx-data.ps1 -AutoResetOnChecksum
```

You can also run the VS Code task “sqlx: regenerate offline metadata”, which executes the same script with the correct working directory.
