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

## Daily Dev Loop

- Infra only (fast inner loop): `./Makefile.ps1 Start-Infra`
- Build all: `./Makefile.ps1 Build-Services`
- Run one service natively: `./Makefile.ps1 Run-Service -Name <service>`
- Run tests (unit): `pushd services; cargo test --workspace; popd`
- Run integration tests: see tests/integration.md and enable with `--features integration`.
- Update SQLx offline metadata after changing macro queries: `./regenerate-sqlx-data.ps1 -Prune`

## Services & Ports (default Compose)

- auth-service: http://localhost:8085
- integration-gateway: http://localhost:8083
- product-service: http://localhost:8081
- order-service: http://localhost:8084
- payment-service: http://localhost:8086
- inventory-service: http://localhost:8087
- loyalty-service: http://localhost:8088
- customer-service: http://localhost:8089
- Kafka UI: http://localhost:8080
- Prometheus: http://localhost:9090
- Grafana: http://localhost:3002 (admin/admin)

All services expose `/healthz` and most expose `/metrics` (Prometheus format) once started.

## Databases & Migrations

- Shared dev DSN (default): `postgres://novapos:novapos@localhost:5432/novapos`
- Run all service migrations: `./migrate-all.ps1`
- Reset database + regenerate SQLx (dev only): `./regenerate-sqlx-data.ps1 -Prune -ResetDatabase`

See development/sqlx-offline.md for per-query SQLx metadata workflow, pruning, and CI patterns.

## Testing

- Workspace tests: `pushd services; cargo test --workspace; popd`
- Integration tests (opt-in): `cargo test --manifest-path services/Cargo.toml --features integration`
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
- Windows Kafka link errors: development/windows-kafka-build.md
- Metrics not visible: bring up Prometheus/Grafana and confirm scrape targets; see security/prometheus-grafana-bootstrap.md

## Helpful Scripts

- `./Makefile.ps1` — Start/Stop infra, build, test, run service, frontends
- `./migrate-all.ps1` — Apply migrations for all services
- `./regenerate-sqlx-data.ps1` — Generate per-query SQLx offline metadata (.sqlx/) with prune/reset options

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
