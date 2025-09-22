# NOVAPOS Security Hardening Addendum (Rust Implementation)

## Purpose
This addendum translates the "Security and compliance plan" into actionable steps for the Rust monorepo. It introduces reusable crates, assigns each service the integration work it must do, and sequences database/infrastructure migrations so engineers can stage changes safely.

## New Workspace Crates
| Crate | Location (proposed) | Purpose | Key Responsibilities | Consumer Services |
| --- | --- | --- | --- | --- |
| `common-auth` | `services/common/auth` | Centralizes JWT verification + tenant & role context handling. | Provide Axum/Tower middleware for RS256 JWT validation, `X-Tenant-ID ↔ claim` checks, refresh-token verification helpers, extractor utilities for `UserContext`. | All HTTP-facing services (product, order, inventory, loyalty, customer, integration-gateway, analytics, payment) |
| `common-rbac` | `services/common/rbac` | Fine-grained role/permission gates usable in handlers. | Define `require_role` / `require_permission` guard functions, policy macros, and test helpers. | Same set as above; especially product, order, inventory, integration-gateway |
| `common-audit` | `services/common/audit` | Shared Kafka producer + structured event schema. | Provide `AuditEvent` structs, JSON encoding, batching helper around `rdkafka`, optional Postgres fallback writer. | All services emitting audit events; `audit-service` consumer |
| `common-crypto` | `services/common/crypto` | Envelope encryption routines for PII and API key hashing. | AES-256-GCM encrypt/decrypt using tenant DEKs, deterministic hashing helpers, Argon2id API key hashing utilities. | customer-service, auth-service, integration-gateway, any PII-handling service |
| `common-config` | `services/common/config` | Typed environment/config loading. | Wrap `config` crate + serde to enforce presence of security-critical env vars (issuer, JWKS URL, master key, Kafka topics). | All services |
| `audit-service` | `services/audit-service` | New service consuming Kafka audit topic(s) and persisting append-only logs. | Consume `audit.events.v1`, write tamper-evident chain into Postgres (`audit_logs`), expose health/metrics. | Deploy as standalone service |

> Add each crate to the workspace by updating `services/Cargo.toml` and publish reusable `lib.rs` APIs. Use feature flags (`integration-tests`, `redis-rate-limit`, etc.) where helpful.

## Service Integration Checklist
### Auth Service (`services/auth-service`)
- Replace UUID login token issuance with RS256 JWT minting using `common-auth::Signer`.
- Expose JWKS endpoint backed by `auth_signing_keys` table; implement key rotation CLI (`rotate-keys.rs`).
- Implement refresh-token store using `auth_refresh_tokens` table and Argon2id hashing for API keys via `common-crypto`.
- Add MFA enrollment/verification endpoints; persist secrets + counters with new migration (see order below).
- Publish audit events for sign-in, MFA, tenant/user admin actions.

### Product Service (`services/product-service`)
- Inject `common-auth` middleware; remove trust in opaque `Authorization` header.
- Apply `common-rbac::require_role("manager")` to create/update/delete routes.
- Replace ad-hoc audit inserts with `common-audit::emit_event` to Kafka (`audit.events.v1`).

### Order Service
- Enforce JWT + tenant binding from `common-auth`.
- Guard refund/cancel endpoints with RBAC helpers.
- Emit audit events for order state transitions.

### Customer Service
- Adopt envelope encryption via `common-crypto` for PII columns; update repository layer to decrypt on read.
- Store per-tenant DEKs in `tenant_data_keys`; pass encrypted payloads through Kafka events.
- Implement GDPR export/delete flows with audit + tombstone writes.

### Integration Gateway
- Switch API key auth to `common-auth` extractor that validates hashed keys from auth-service.
- Replace in-memory rate limiting with Redis-backed limiter (using `redis` crate) and publish usage audits.
- Verify Coinbase webhooks using shared crypto utils; emit partner activity metrics.

### Inventory, Loyalty, Analytics, Payment Services
- Install `common-auth` middleware for JWT validation + tenant context.
- Use `common-rbac` to protect write endpoints.
- Emit baseline audit events (config changes, rewards issuance, payout triggers, report exports).

### Frontends / Admin Portal
- Update login flow to handle JWT + refresh tokens, MFA prompts, and new error codes.
- Persist tokens securely (httpOnly cookies or secure storage) and send `Authorization: Bearer` headers.

## Database & Infrastructure Migrations
Apply through existing sqlx workflow; create migration files in each service’s `migrations/` directory.

1. **Auth Service**
   1. `3006_create_auth_signing_keys_rs256.sql` – schema matches plan (kid, PEMs, active flag).
   2. `3007_create_auth_refresh_tokens.sql` – includes indexes on `(user_id, revoked_at)`.
   3. `3008_create_mfa_tables.sql` – store TOTP secrets, recovery codes, telemetry.

2. **Shared (New `services/audit-service`)**
   - `1001_create_audit_logs.sql` – append-only table with hash chaining fields (`prev_hash`, `entry_hash`).

3. **Customer Service**
   1. `5002_add_tenant_keys.sql` – create `tenant_data_keys` table.
   2. `5003_add_customer_encrypted_columns.sql` – add encrypted columns & deterministic hash indexes.
   3. `5004_create_gdpr_tombstones.sql` – track deletions & export state.
   4. Backfill script (`scripts/backfill_customer_pii.rs`) to encrypt historical rows.

4. **Integration Gateway**
   - `2001_add_api_key_usage_table.sql` – store request counters, last seen, derived metrics (optional if Redis is primary source).

5. **Kafka Topics**
   - Provision `audit.events.v1`, `security.mfa.activity`, `gdpr.requests.v1` via existing `kafka-topics-init` job.

> Order migrations so auth JWT infrastructure lands before dependent services flip middleware. Run migrations in lower environments first, validating new crates via integration tests before production rollout.

## Phased Rollout (Rust Focus)
1. **Foundations**
   - Scaffold `common-*` crates + unit tests.
   - Add `auth-service` migrations & JWKS endpoint; deploy without enforcing JWT yet (dual token issuance).

2. **Service Adoption Wave 1**
   - Integration Gateway + Product Service move to `common-auth`; enable RBAC and audit emissions.
   - Launch `audit-service` and confirm Kafka → Postgres flow.

3. **Service Adoption Wave 2**
   - Order, Inventory, Loyalty, Customer, Analytics, Payment services adopt middleware & RBAC.
   - Frontend consumes JWT + refresh flow.

4. **PII & GDPR Enablement**
   - Deploy customer-service encryption migrations, run backfill, then switch API responses to decrypted fields.
   - Activate GDPR export/delete endpoints and admin UI controls.

5. **Hardening & Monitoring**
   - Enable MFA requirement, suspicious activity alerts, Redis rate limiting in integration-gateway.
   - Add Prometheus `/metrics` endpoints to every service; ship dashboards/alerts.

## Wave 5: Hardening & Monitoring

### Rollout Goals
- Enforce strong customer and admin authentication, capture anomalies, and surface signals to responders.
- Harden the integration-gateway perimeter with rate limiting, traceable key usage, and tuned alert thresholds.
- Standardize runtime telemetry so SRE can dashboard, alert, and create runbooks off a consistent metrics/logs footprint.
- Close the loop with post-rollout verification and disaster recovery exercises.

### Auth Service
- Flip the 
equire_mfa feature flag once the mobile/web clients confirm MFA enrollment UX; gate privileged routes (tenant admin, key rotation) behind MFA checks.
- Emit security.mfa.activity Kafka events for enrollment changes, MFA failures, bypass attempts, and suspicious geolocation/device combinations.
- Add rate-limited suspicious login webhooks to the security Slack channel; include tenant, user, ip, device fingerprint, and correlating audit 	race_id.
- Backfill dashboards that track failed login spikes, MFA enrollment coverage, and top tenants by bypass usage.

### Integration Gateway
- Enable the Redis-backed limiter in production with ceilings from GATEWAY_RATE_LIMIT_RPM; expose limiter counters via /metrics (gateway_rate_limit_hits_total, gateway_rate_limit_rejections_total).
- Persist API key usage deltas into pi_key_usage every five minutes and publish hourly summaries to udit.events.v1 with tenant and key prefix tags.
- Alert when a key bursts above three times its seven-day P95 or when signature verification failures exceed ten per minute; route alerts to PagerDuty (P1) and the security Slack channel (P2).

### Fleet-Wide Monitoring
- Confirm every service exports http_requests_total, http_request_duration_seconds, and uth_jwt_validation_failures_total; document label conventions in docs/security/README.md.
- Register audit-service with Prometheus (udit_events_processed_total, udit_chain_gap_total) and add Grafana panels for lag, chain integrity, and Postgres disk usage.
- Stream structured JSON logs to the central log pipeline; include 	race_id, 	enant_id, user_id, and ctor_role fields for cross-service correlation.
- Define SLO monitors covering: 99.5 percent of auth token validations under 200ms, 99 percent of gateway requests avoiding rate limiting, and zero audit chain gaps longer than 60 seconds; wire alert thresholds to on-call rotations.

### Alerting & Runbooks
- Publish playbooks in docs/security/README.md covering MFA lockouts, key compromise response, audit chain gaps, and rate-limit tuning.
- Hook Prometheus alerts into PagerDuty with runbook links and tag the SRE and Security rotations.
- Schedule quarterly disaster recovery tests that rotate signing keys under load, simulate Redis outages, and confirm audit-service catch-up plus alert delivery.
- Add security dashboards that surface MFA adoption, audit retention, and incident response MTTR for compliance evidence.

### Verification Checklist
- All services show /metrics scrape success in Prometheus and dashboard panels populate within five minutes.
- Synthetic login and webhook tests fire and produce the expected Kafka events plus alert notifications.
- Security and SRE leads sign off on updated runbooks and close the Jira tasks linked to Wave 5.
- Update the post-mortem template so it captures MFA and rate-limit mitigations.

## Testing & Tooling Notes
- Add `cargo xtask lint-security` to run clippy + targeted security tests for new crates.
- Provide cucumber/integration tests that spin up JWKS, Redis, Kafka test containers using `testcontainers` crate.
- Document recovery/runbooks in `docs/security/README.md` alongside this addendum.

## Ownership
- Security engineering: crate APIs, audit-service, crypto design.
- Service teams: integrate middleware, run migrations, own feature toggles.
- SRE/Platform: Kafka topics, Redis, monitoring, secrets management.
