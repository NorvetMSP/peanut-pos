# NovaPOS From-Scratch Dev Environment Bootstrap (Windows / PowerShell Focus)

> Goal: A single, authoritative checklist to go from an empty machine (with Docker & Rust toolchain installed) to a fully running local NovaPOS stack (services + optional frontends + monitoring) with reproducible SQLx offline metadata.

---

## 0. Supported Host Environment

Tested on:

- Windows 11, PowerShell 5.1 / PowerShell 7
- Docker Desktop (>= 4.30) with Linux containers

Should also work (with command translation) on macOS/Linux; adapt paths & package managers.

## 1. Prerequisites (Install Once)

| Component | Purpose | Suggested Install |
|-----------|---------|-------------------|
| Git | Clone repo | winget install Git.Git |
| Rust toolchain | Build Rust services | [rustup.rs](https://rustup.rs) (ensure latest stable) |
| SQLx CLI | DB create & migrations & offline prepare | `cargo install sqlx-cli --no-default-features --features postgres` |
| Node.js 20+ | Frontend dev (Vite, React) | [nodejs.org](https://nodejs.org) (LTS) |
| npm (bundled) | Frontend deps | ships with Node |
| Docker Desktop | Local infra (PG, Redis, Kafka, Vault) | [docker.com](https://www.docker.com) |
| curl | Healthchecks / quick tests | winget install curl |
| (Optional) psql | Reset / terminate sessions (regeneration script) | Ship via postgres or install `winget install PostgreSQL.PostgreSQL` |

Rust components automatically pulled:

- Cargo, rustc stable
- For Kafka (rdkafka) on Windows using MSVC: Visual Studio Build Tools (C++), CMake (if not present) — install via `winget install Kitware.CMake` & VS Build Tools with C++ workload.

## 2. One-Time Git Clone

```powershell
cd C:\Projects
git clone https://github.com/datawrangler05/novapos.git
cd novapos
```

(If using SSH, replace URL accordingly.)

## 3. Environment Overview

Two docker-compose contexts:

1. `local/docker-compose/docker-compose.yml` – minimal infra only (Postgres, Redis, Kafka) for iterative dev running services natively via `cargo run`.
2. Root `docker-compose.yml` – full stack: infra + all Rust services + frontends + monitoring (Prometheus/Grafana) + Vault + Kafka UI.

Pick the mode that suits the workflow:

- Fast inner loop (code/run/debug): Minimal infra + `cargo run` individual service.
- Full integration: Root compose to see everything interact.

## 4. Fresh Machine Bootstrap (Happy Path)

### 4.1 Start Minimal Infra (recommended first run)

```powershell
./Makefile.ps1 Start-Infra
```

Verifies Docker works and exposes services on:

- Postgres: 5432
- Redis: 6379
- Kafka: 9092

### 4.2 Build All Rust Services

```powershell
./Makefile.ps1 Build-Services
```

(Alternatively: `pushd services; cargo build --workspace; popd`)

If Windows + Kafka linking errors (`__imp_crc32` etc.) occur, see Section 10.

### 4.3 Run All Migrations (Against dev DB)

Uses shared DB (`novapos`) for all services.

```powershell
# Sets DATABASE_URL internally and migrates each service that has migrations
./migrate-all.ps1
```

### 4.4 Generate / Refresh SQLx Offline Metadata

Prune to drop stale files, capture fresh macros:

```powershell
# Common first run
./regenerate-sqlx-data.ps1 -Prune
```

Options:

- Add `-Features integration` if you need feature-gated queries.
- Add `-Diff` to see additions/removals.

### 4.5 Launch One Service For Dev

```powershell
./Makefile.ps1 Run-Service -Name auth-service
```

Add a new terminal per service or rely on full compose later.

### 4.6 (Optional) Frontend Dev

```powershell
./Makefile.ps1 Dev-Frontend -App admin-portal
# Or pos-app when present
```

Serves on Vite dev port (see console output, usually 5173 or configured alternative).

### 4.7 Full Stack (Alternative Path)

Instead of steps 4.1–4.6 you can directly:

```powershell
docker compose up --build
```

This builds each service image (Dockerfile per service) and starts everything including Vault, Kafka UI, Prometheus, Grafana, and frontends.

Ports (host):

- Auth: 8085
- Product: 8081
- Order: 8084
- Inventory: 8087
- Customer: 8089
- Loyalty: 8088
- Payment: 8086
- Integration Gateway: 8083
- Analytics: 8082
- POS App: 3000
- Admin Portal: 3001
- Kafka UI: 8080
- Prometheus: 9090
- Grafana: 3002
- Vault: 8200

### 4.8 Verify JWT Keys (Auth Service)

Root compose mounts `jwt-dev.pem` & `jwt-dev.pub.pem` as secrets; when running services locally outside compose you must export paths or env vars if required (currently mostly compose-managed). For local native runs with dev keys accessible, set:

```powershell
$env:JWT_DEV_PRIVATE_KEY_PEM_FILE = (Resolve-Path .\jwt-dev.pem)
$env:JWT_DEV_PUBLIC_KEY_PEM_FILE  = (Resolve-Path .\jwt-dev.pub.pem)
```

### 4.9 Health Check Examples

```powershell
curl http://localhost:8085/.well-known/jwks.json
curl http://localhost:8081/healthz
```

## 5. Daily Dev Reset / Clean Cycle

Typical morning routine:

```powershell
git pull
./regenerate-sqlx-data.ps1 -Prune  # pick up schema/query changes
./Makefile.ps1 Start-Infra          # if not already running
./Makefile.ps1 Run-Service -Name product-service
```

If schema changes:

```powershell
./migrate-all.ps1
./regenerate-sqlx-data.ps1 -Prune
```

## 6. Adding / Modifying a Query



Use if metadata drift, compiled artifacts corruption, or major tool upgrades.

```powershell
# Stop everything
./Makefile.ps1 Stop-Infra
# Remove target artifacts
Remove-Item -Recurse -Force .\services\target -ErrorAction SilentlyContinue
# Clean cargo (workspace-level)
pushd services; cargo clean; popd
# Drop DB & start infra again
./Makefile.ps1 Start-Infra
# Migrations & offline metadata fresh
./Makefile.ps1 Build-Services
```

Then either run services individually or `docker compose up --build`.

## 8. Choosing Compose Mode vs Native Run

| Scenario | Recommendation |
|----------|---------------|
| Iterating on single service logic | Minimal infra + `Run-Service` |
| Testing inter-service Kafka flows | Root `docker-compose.yml` full stack |
| Working on frontends consuming multiple APIs | Full stack |
| Debugging DB migrations quickly | Minimal infra + local runs |

## 9. Common Environment Variables (Local Defaults)

| Variable | Purpose | Typical Dev Value |
|----------|---------|-------------------|
| DATABASE_URL | Shared Postgres URL | `postgres://novapos:novapos@localhost:5432/novapos` or service-scope variant inside compose |
| KAFKA_BOOTSTRAP | Kafka | `localhost:9092` (minimal) or `kafka:9092` (compose) |
| REDIS_URL | Redis cache (integration-gateway) | `redis://localhost:6379/0` |
| JWT_ISSUER | JWT claims | `https://auth.novapos.local` |
| JWT_AUDIENCE | Audience list | `novapos-frontend,novapos-admin,novapos-postgres` |
| VAULT_ADDR / VAULT_TOKEN | Secrets (when enabled) | `http://localhost:8200` / `root` |

Frontends typically configure base API URLs via environment or config; ensure they point to exposed host ports.

## 10. Windows Kafka (Linking) Quick Reference

If you encounter unresolved zlib symbols during `cargo build` for a Kafka-enabled service (e.g. `auth-service`):

- See `docs/development/windows-kafka-build.md` for full explanation.
- Ensure Visual C++ Build Tools installed.
- Confirm build script emits lines containing `cargo:rustc-link-lib=zlib`.
 
```powershell
cargo clean -p auth-service
cargo build -p auth-service -vv
```

If still failing, delete `target\\build\\rdkafka-sys-*` directories and rebuild.

## 11. Troubleshooting Matrix

| Symptom | Likely Cause | Fix |
|---------|--------------|-----|
| `sqlx offline artifact not produced` | Skipped prepare after adding macro | Re-run `./regenerate-sqlx-data.ps1 -Prune` |
| Migrations checksum mismatch error | Edited applied migration | Use `-ResetDatabase` (dev) OR create new follow-on migration |
| Service cannot connect to DB | Infra not started or wrong `DATABASE_URL` | `./Makefile.ps1 Start-Infra` then retry |
| Kafka topics missing | Startup race before Kafka ready | Restart dependent services or use root compose (has healthchecks) |
| Frontend CORS issues | Wrong API base URL or missing headers | Confirm service health endpoints reachable from browser |
| Vault errors during compose | Vault container not yet initialized | Wait a few seconds; dev token is static (`root`) |

## 12. Quick Command Cheatsheet

```powershell
# Infra only (fast inner loop)
./Makefile.ps1 Start-Infra
./Makefile.ps1 Stop-Infra

# Build / test everything
./Makefile.ps1 Build-Services
./Makefile.ps1 Test-Services

# Migrations & SQLx offline
./migrate-all.ps1
./regenerate-sqlx-data.ps1 -Prune

# Run individual service
./Makefile.ps1 Run-Service -Name product-service

# Full stack (all services + frontends + monitoring)
docker compose up --build

# Tear down full stack (but keep volumes)
docker compose down

# Full reset DB + metadata
./regenerate-sqlx-data.ps1 -Prune -ResetDatabase
```

## 14. Payment Void Endpoint & Event (Integration Gateway)

The Integration Gateway exposes a `POST /payments/void` endpoint used by internal tooling or support roles to void an in-flight or recently authorized payment before settlement.

Request (JSON):

```json
{
  "orderId": "<uuid>",
  "method": "card|cash|other",
  "amount": 42.15,
  "reason": "duplicate" // optional
}
```

Required Headers:

- `X-Tenant-ID: <uuid>`
- `X-User-ID: <uuid>` (actor performing the void)
- `X-Roles: support` (must include a role with void capability)

Success Response:

```json
{ "status": "voided" }
```

### Event Emission

When compiled with the `kafka` or `kafka-producer` feature, a `payment.voided` event is produced after the operation succeeds.

Topic: (configured) e.g. `payment.events.v1` (actual topic name may vary; audit topic configured via `GATEWAY_AUDIT_TOPIC`)

Example Payload (conceptual – align with downstream schema registry if present):

```json
{
  "type": "payment.voided",
  "order_id": "<uuid>",
  "tenant_id": "<uuid>",
  "method": "card",
  "amount": 42.15,
  "reason": "duplicate",
  "occurred_at": "2024-07-04T12:34:56.789Z"
}
```

If the service is built without Kafka features, the endpoint still returns success but no event is emitted (important for local lightweight dev loops without a broker).

### Test Capture Hook

For future automated tests, setting the environment variable `TEST_CAPTURE_KAFKA=1` enables an internal capture path inside the `void_payment` handler (Kafka feature builds only). This stores emitted records in a test-only static for assertion. The current test suite keeps only a build smoke test; the full event assertion harness is deferred.

### Failure Modes

- 400: Validation / malformed body
- 403: Missing required role/capability
- 404: Order not found (future enhancement; presently may still return success if logic is mocked)
- 500: Unexpected internal error (should be rare; check logs)

### Operational Notes

- Ensure Kafka broker reachable at `KAFKA_BOOTSTRAP` when features enabled; otherwise startup will fail early.
- For local dev without Kafka, omit the feature flags to speed build & avoid broker dependency.
- Metrics and audit usage trackers run background tasks; watch logs for flush intervals to confirm healthy timers.


## 13. Future Improvements (TODO)

- Script: seed baseline demo data (products, sample customers) via a `seed-data.ps1`.
- Add `cargo nextest` for faster test feedback.
- Add Windows CI matrix including a Kafka build validation.
- Adopt `.env` file for unified variable injection.

---
**Single-Line TL;DR (First Time):**

```powershell
git clone https://github.com/datawrangler05/novapos.git; cd novapos; ./Makefile.ps1 Start-Infra; ./migrate-all.ps1; ./regenerate-sqlx-data.ps1 -Prune; ./Makefile.ps1 Run-Service -Name auth-service
```

This document is the canonical source for fresh dev bootstrap. Update it whenever build or infra assumptions change.
