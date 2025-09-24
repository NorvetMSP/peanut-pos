# Test Suite Overview

This document tracks the automated test coverage across the NovaPOS services. As we expand the suite, update the tables below and include any new commands or noteworthy fixtures.

## How to Run Everything

```
cargo test --workspace
```

> The command above executes every Rust crate in the monorepo. Individual crates and binaries can be targeted via `-p <crate-name>`.

## Current Coverage

| Area | Location | Test Types | Notes |
| --- | --- | --- | --- |
| Shared Auth Library | `services/common/auth` | Unit tests | Covers JWT claims parsing, bearer extractor validation, in-memory key store behaviour, and JWKS refresh paths. Added in Sept 2025 to prevent regressions in auth-service consumers. |
| Auth Service | `services/auth-service` | Unit tests | Protects cookie/session helpers, tenant header parsing, and password hashing. Forms the base for deeper handler integration tests. |
| Auth Service (Login Flow) | `services/auth-service/tests/login_flow.rs` | Integration (ignored) | Spins up embedded Postgres with pg-embed (downloads ~200MB) or reuse an existing instance by setting `AUTH_TEST_DATABASE_URL`. Run with `cargo test -p auth-service --test login_flow -- --ignored`. |

## Coverage Roadmap

These are the immediate focus areas for expanding coverage. Add details or mark items complete as we build them out.

- [ ] `auth-service` handler tests exercising login, refresh, logout, and MFA flows with an in-memory Postgres fixture. Happy-path login runs in `tests/login_flow.rs` (ignored by default until the embedded Postgres story is battle-tested); next step is broadening coverage and stabilising the fixture.
- [ ] End-to-end smoke tests that stand up a minimal stack (`auth-service` + dependencies) to validate routing and telemetry wiring.
- [ ] Negative-path tests for token issuance covering missing signing keys and revoked refresh tokens.
- [ ] Load-shedding scenarios for JWKS refresh (network failures, malformed payloads) promoted to integration tests.

## Contribution Checklist

When adding new tests:

1. Update this document with the area, path, and a short description.
2. Include commands or scripts required to execute the new tests.
3. Note any shared fixtures or helpers so other teams can reuse them.
4. If the tests require new tooling or dependencies, document installation steps in the relevant service README.






