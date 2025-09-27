# Test Suite Overview

This document tracks the automated test coverage across the NovaPOS services. As we expand the suite, update the tables below and include any new commands or noteworthy fixtures.

## How to Run Everything

```bash
cd C:\Projects\novapos\services
$env:PATH  = "C:\vcpkg\installed\x64-windows\bin;$env:PATH"
$env:LIB   = "C:\vcpkg\installed\x64-windows\lib;$env:LIB"
$env:INCLUDE = "C:\vcpkg\installed\x64-windows\include;$env:INCLUDE"
$env:VCPKGRS_DYNAMIC = "1"
cd  C:\Projects\novapos\services cargo test --workspace

$env:AUTH_TEST_USE_EMBED = "1"
$env:AUTH_TEST_APPLY_MIGRATIONS = "1"   
$env:AUTH_TEST_EMBED_CLEAR_CACHE = "1" 
```

> The command above executes every Rust crate in the monorepo. Individual crates and binaries can be targeted via `-p <crate-name>`.
>
> Auth-service integration tests default to the Docker Compose Postgres DSN (`postgres://novapos:novapos@localhost:5432/novapos`). Override it with `AUTH_TEST_DATABASE_URL`, or set `AUTH_TEST_USE_EMBED=1` if you prefer the embedded Postgres helper.

## Current Coverage

| Area | Location | Test Types | Notes |
| --- | --- | --- | --- |
| Shared Auth Library | `services/common/auth` | Unit tests | Covers JWT claims parsing, bearer extractor validation, in-memory key store behaviour, and JWKS refresh paths. Added in Sept 2025 to prevent regressions in auth-service consumers. |
| Auth Service | `services/auth-service` | Unit tests | Protects cookie/session helpers, tenant header parsing, and password hashing. Forms the base for deeper handler integration tests. |
| Auth Service (Login Flow) | `services/auth-service/tests/login_flow.rs` | Integration (ignored) | Uses the Docker Compose Postgres by default (`postgres://novapos:novapos@localhost:5432/novapos`). Override with `AUTH_TEST_DATABASE_URL` or set `AUTH_TEST_USE_EMBED=1` for the embedded helper. Optional flags: `AUTH_TEST_EMBED_CLEAR_CACHE=1`, `AUTH_TEST_APPLY_MIGRATIONS=1`. Run with `cargo test -p auth-service --test login_flow -- --ignored`. |
| Auth Service (Stack Smoke) | `services/auth-service/tests/stack_smoke.rs` | Integration (ignored) | Launches the compiled binary against Postgres (defaults to the Compose DSN) and Kafka. Verifies health, login, session refresh, and teardown. Run with `cargo test -p auth-service stack_smoke_happy_path -- --ignored --nocapture`. |
| Auth Service (Token Signer) | `services/auth-service/tests/token_signer.rs` | Integration (ignored) | Exercises missing signing key failure, JWKS fallback, and refresh-token reuse. Same env flags as login flow; run with `cargo test -p auth-service --test token_signer -- --ignored`. |
| Auth Service (Axum Smoke) | `services/auth-service/tests/axum_smoke.rs` | Integration (ignored) | Boots an Axum router wired like the production service (including `/mfa/enroll` and `/mfa/verify`) and points at the Compose Postgres by default. Exercises `/healthz`, `/metrics`, `/login` flows (MFA negative/positive), `/session`, `/logout`, tenant integration-key admin, and verifies Kafka events (primary + DLQ) under simulated failures. Run with `cargo test -p auth-service --test axum_smoke -- --ignored --nocapture`. |
| Auth Service (JWKS Load-Shedding) | `services/auth-service/tests/jwks_loadshed.rs` | Integration | Simulates JWKS refresh errors/timeouts and ensures cached keys remain valid. Run with `cargo test -p auth-service --test jwks_loadshed`. |

### Example: Auth Service Login Flow

If the Docker Compose Postgres container is running, you can execute the ignored login flow suite directly:

```powershell
cargo test -p auth-service --test login_flow -- --ignored
```

### Example: Auth Service Smoke Tests

```powershell
cargo test -p auth-service stack_smoke_happy_path -- --ignored --nocapture
cargo test -p auth-service --test axum_smoke -- --ignored --nocapture
```

To target a different database, set `AUTH_TEST_DATABASE_URL=postgres://user:pass@host:port/dbname` before invoking `cargo test`. To fall back to the embedded Postgres helper instead, export `AUTH_TEST_USE_EMBED=1` (and optionally `AUTH_TEST_APPLY_MIGRATIONS=1` and `AUTH_TEST_EMBED_CLEAR_CACHE=1` for a fresh cache).

For stack_smoke and other Kafka-dependent tests, point to your broker first (default assumes `localhost:19092`):

```powershell
set AUTH_TEST_KAFKA_BOOTSTRAP=localhost:9092
```

## Coverage Roadmap

- [ ] `auth-service` handler tests covering lockout thresholds, suspicious webhook fallbacks, and explicit role gating (built on the existing login/stack/axum suites).
- [ ] End-to-end smoke tests that stand up a minimal stack (`auth-service` + dependencies) to validate routing and telemetry wiring.
- [x] Negative-path tests for token issuance covering missing signing keys and revoked refresh tokens (see `services/auth-service/tests/token_signer.rs`).
- [x] Load-shedding scenarios for JWKS refresh (see `services/auth-service/tests/jwks_loadshed.rs`).

## Contribution Checklist

When adding new tests:

1. Update this document with the area, path, and a short description.
2. Include commands or scripts required to execute the new tests.
3. Note any shared fixtures or helpers so other teams can reuse them.
4. If the tests require new tooling or dependencies, document installation steps in the relevant service README.
## Remaining Test Work

- Auth service: add focused handler/unit integration cases for lockout boundaries, role-based access, and webhook failure retries (building on the shared fixtures).
- Compose-level end-to-end suite that wires multiple services together (auth-service + API gateway) to validate routing, authn/authz headers, and telemetry.
- Non-auth services: expand smoke coverage for inventory/order/payment services once their fixtures mirror the auth harness (database seeds, Kafka stubs, etc.).




