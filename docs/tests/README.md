# Test Suite Overview

This document tracks the automated test coverage across the NovaPOS services. As we expand the suite, update the tables below and include any new commands or noteworthy fixtures.

## How to Run Everything

```bash
cargo test --workspace
```

> The command above executes every Rust crate in the monorepo. Individual crates and binaries can be targeted via `-p <crate-name>`.

## Current Coverage

| Area | Location | Test Types | Notes |
| --- | --- | --- | --- |
| Shared Auth Library | `services/common/auth` | Unit tests | Covers JWT claims parsing, bearer extractor validation, in-memory key store behaviour, and JWKS refresh paths. Added in Sept 2025 to prevent regressions in auth-service consumers. |
| Auth Service | `services/auth-service` | Unit tests | Protects cookie/session helpers, tenant header parsing, and password hashing. Forms the base for deeper handler integration tests. |
| Auth Service (Login Flow) | `services/auth-service/tests/login_flow.rs` | Integration (ignored) | Spins up embedded Postgres (`AUTH_TEST_USE_EMBED=1`) or reuse an existing instance via `AUTH_TEST_DATABASE_URL`. Optional flags: `AUTH_TEST_EMBED_CLEAR_CACHE=1`, `AUTH_TEST_APPLY_MIGRATIONS=1`. Run with `cargo test -p auth-service --test login_flow -- --ignored`. |
| Auth Service (Token Signer) | `services/auth-service/tests/token_signer.rs` | Integration (ignored) | Exercises missing signing key failure, JWKS fallback, and refresh-token reuse. Same env flags as login flow; run with `cargo test -p auth-service --test token_signer -- --ignored`. |
| Auth Service (Axum Smoke) | `services/auth-service/tests/axum_smoke.rs` | Integration (ignored) | Boots a Router with embedded Postgres fixtures to exercise `/healthz`, `/metrics`, `/login`, `/session`, `/logout`, and tenant integration-key admin routes. Run with `cargo test -p auth-service --test axum_smoke -- --ignored`. |

## Coverage Roadmap

- [ ] `auth-service` handler tests exercising login, refresh, logout, and MFA flows with an in-memory Postgres fixture. Happy-path login runs in `tests/login_flow.rs` (ignored by default until the embedded Postgres story is battle-tested); new smoke coverage lives in `tests/axum_smoke.rs`; next up is layering in MFA failure modes and webhook assertions to harden the fixture.
- [ ] End-to-end smoke tests that stand up a minimal stack (`auth-service` + dependencies) to validate routing and telemetry wiring.
- [x] Negative-path tests for token issuance covering missing signing keys and revoked refresh tokens (see `services/auth-service/tests/token_signer.rs`).
- [ ] Load-shedding scenarios for JWKS refresh (network failures, malformed payloads) promoted to integration tests.

## Contribution Checklist

When adding new tests:

1. Update this document with the area, path, and a short description.
2. Include commands or scripts required to execute the new tests.
3. Note any shared fixtures or helpers so other teams can reuse them.
4. If the tests require new tooling or dependencies, document installation steps in the relevant service README.
