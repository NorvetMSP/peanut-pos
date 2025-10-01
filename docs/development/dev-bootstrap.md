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

1. Edit Rust source using `query!` or `query_as!` macro.
2. Run: `./regenerate-sqlx-data.ps1 -Prune` (or omit -Prune if just additive).
3. Commit changed `.sqlx/query-*.json` files.
4. Build with `env:SQLX_OFFLINE=1 cargo build --workspace` as a validation (CI replicates this).

## 7. Full Rebuild From Absolute Scratch (Nuclear Option)

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
./migrate-all.ps1
./regenerate-sqlx-data.ps1 -Prune -ResetDatabase
# Rebuild
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
- Retry verbose build:

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
| JWT validation failing in services | Auth not healthy or keys not mounted | Check `curl http://localhost:8085/.well-known/jwks.json` |
| 808x port already in use | Stale previous process | `Get-Process -Id (netstat -ano findstr :8085)` and stop offending PID |
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
