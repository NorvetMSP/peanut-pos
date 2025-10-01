# Integration Tests

This repository uses a Cargo feature flag named `integration` to control execution of slow, infrastructure-dependent tests (Postgres, Kafka, launching full service stacks, embedded Postgres).

All such tests are declared with:

```rust
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires infra)")]
```

So they remain visible (`cargo test -- --ignored`) but are only executed when the feature is enabled.

---

## Quick Start (Local Dev)

1. Ensure required infrastructure is running (at minimum Postgres; some auth tests also expect Kafka & Redis):

```powershell
docker compose up -d postgres kafka redis
```

1. (Optional) Prepare a database / apply migrations for all services:

```powershell
powershell -ExecutionPolicy Bypass -File .\migrate-all.ps1
```

1. Run integration tests for the whole workspace (from repo root):

```powershell
cargo test --manifest-path services/Cargo.toml --features integration
```

1. To run only one crate's integration tests:

```powershell
cargo test --manifest-path services/Cargo.toml -p auth-service --features integration
```

1. To list which tests are gated (without running them):

```powershell
cargo test --manifest-path services/Cargo.toml -- --ignored --list
```

---

## Embedded vs External Postgres (auth-service)

`auth-service` tests can spin up an embedded Postgres (pg-embed) if environment variables are set to permit it or fall back to a default Docker DSN. For faster iterations you can simply rely on the dockerized Postgres from `docker compose`.

Common variables:

- `AUTH_TEST_DATABASE_URL` (explicit DSN) OR
- `AUTH_TEST_USE_EMBED=1` (attempt embedded Postgres) + optionally `AUTH_TEST_EMBED_CLEAR_CACHE=1` to clear cached binaries.

Migrations now auto-apply in these cases:

- Embedded Postgres (`AUTH_TEST_USE_EMBED=1`)
- Explicit opt-in (`AUTH_TEST_APPLY_MIGRATIONS=1`)
- Default docker DSN (the harness detects the built-in `postgres://novapos:novapos@localhost:5432/novapos` URL)

Opt out with:

```powershell
$env:AUTH_TEST_SKIP_AUTO_MIGRATIONS = "1"
```

If you need to re-run only a subset manually, you can still invoke `sqlx migrate run` yourself beforehand.

Idempotency: The auth-service test harness tolerates re-running migrations; benign "already exists" create errors are logged and ignored so repeated integration test runs against a shared local Postgres won't fail.

### Refresh Token Revocation Strategy

Refresh tokens are single-use. The `consume_refresh_token` function now performs a `SELECT ... FOR UPDATE` followed by a hard `DELETE` of the matched row instead of updating a `revoked_at` column. This was changed to:

- Avoid schema drift / missing `revoked_at` column causing 500s in older environments.
- Guarantee one-time semantics without relying on soft-revoke metadata.

Implications:

- No historical audit trail of refresh token revocations by default.
- Reuse attempts simply appear as `session_expired` (401) since the row is gone.
- If future auditing is needed, we can reintroduce soft revocation behind a feature flag or log sink.

Dedicated micro test `session_flow` validates: login → refresh OK → logout → refresh 401 for fast feedback without running the full smoke suite.

---

## Why a Feature Flag Instead of #[ignore]

Previously tests were permanently `#[ignore]`, which hid them from normal workflows. The feature flag approach provides:

- Discoverability (they appear in `cargo test -- --ignored` output)
- Opt-in execution without editing source
- Consistent gating across all services

---

## Guidelines for Adding New Integration Tests

1. Write the test with normal `#[tokio::test]` (or other harness attribute).
2. Add the gating attribute directly after it:

```rust
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires <resource>)")]
```

1. Avoid leaking external state: create tenants, users, etc., uniquely (UUIDs) and clean up if practical.
1. Prefer shared helpers (see `auth-service/tests/support`).
1. Keep fast unit tests (pure logic, no network/database) outside this gating to maintain quick feedback cycles.

---

## CI Strategy (Planned)

A future GitHub Actions workflow (`integration-tests.yml`) will:

1. Launch required services via docker compose (Postgres, Kafka, Redis).
2. Run migrations.
3. Execute:

```bash
cargo test --manifest-path services/Cargo.toml --features integration --all-targets --no-fail-fast
```

1. Archive logs / test reports as artifacts.

---

## Troubleshooting

| Symptom | Likely Cause | Fix |
|---------|--------------|-----|
| Tests immediately skip with "enable with --features integration" | Feature flag not enabled | Add `--features integration` |
| Connection refused errors | Infra containers not running | `docker compose ps` then `docker compose up -d <service>` |
| Auth tests stuck waiting for migrations | Missing DB or wrong DSN | Ensure `postgres` container healthy & `DATABASE_URL` exported |
| Kafka related failures | Kafka not started | `docker compose up -d kafka` |

---

## Example Targeted Run

Only run auth-service integration tests verbosely:

```powershell
cargo test --manifest-path services/Cargo.toml -p auth-service --features integration -- --nocapture
```

---

## Future Enhancements

- Separate feature flags (e.g. `integration-db`, `integration-kafka`) if runtime becomes too long.
- JUnit XML output for CI artifacts.
- Parallel dockerized workflow for PR gating.

---

Maintainers: Update this doc whenever integration test infrastructure assumptions change.
