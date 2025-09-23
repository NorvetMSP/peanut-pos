# Wave 5 Follow-On Ticket Backlog

Use these ticket outlines when scheduling the remaining Wave 5 security work. Each item includes dependencies, key tasks, and concrete acceptance criteria so the owning team can move straight into execution.

## 1. Audit-Service Rollout
**Goal**: Deploy the Kafka-backed audit-service and make its telemetry visible for security/SRE review.

- **Dependencies**
  - Secrets promoted to staging/prod (`KAFKA_BOOTSTRAP`, DB credentials).
  - Kafka topics `audit.events.v1` and DLQ created with retention matching the addendum.
  - Prometheus/Grafana stack online (per `docs/security/README.md`).
- **Key Tasks**
  - Stand up the `services/audit-service` deployment (Helm/ECS) in staging, then production.
  - Apply database migrations for `audit_logs` tables with chaining columns.
  - Configure service discovery so Prometheus scrapes `/metrics` and Grafana panels cover lag/chain integrity.
  - Enable Kafka producers in auth-service and other emitters to publish audit events and verify round-trip.
  - Backfill historical audit events if required (publish from archived logs or recent auth events).
  - Run load + failover smoke: simulate burst load, restart service, confirm chain continuity.
- **Acceptance Criteria**
  - Audit-service pods/tasks healthy in staging & prod with autoscaling tuned.
  - Kafka topic shows sustained throughput with <10s end-to-end lag under nominal load.
  - Prometheus exposes `audit_events_processed_total` and `audit_chain_gap_total`; Grafana dashboard displays them.
  - At least one signed sample audit record captured in prod, and runbook updated with incident triage steps.

## 2. Fleet JWKS Adoption
**Goal**: Ensure every HTTP service verifies tokens via the shared `common-auth` JWKS flow.

- **Dependencies**
  - Auth-service JWKS endpoint live (`/.well-known/jwks.json`).
  - `common-auth` crate published/consumed by target services.
- **Key Tasks**
  - Inventory services still using legacy token verification or shared secrets.
  - Update each service to import `common-auth` middleware and remove inline signature logic.
  - Configure JWT issuer/audience via `common-config`; toggle old env vars off.
  - Add integration tests per service hitting JWKS fetch/rotation paths (use testcontainers where possible).
  - Simulate key rotation using `auth-service` CLI and confirm services reload without restart.
  - Update service runbooks to reference JWKS dependency & rotation steps.
- **Acceptance Criteria**
  - All services confirm via configuration diff that `common-auth` provides request guards.
  - Automated test or CI job exercises JWKS rotation path.
  - Legacy JWT secrets removed from repos and secret stores.
  - Post-rotation smoke in staging passes without downtime.

## 3. Disaster Recovery Drills
**Goal**: Institutionalise quarterly DR tests covering key Wave 5 controls.

- **Dependencies**
  - Monitoring/alerting wired (Grafana & PagerDuty) so drills observe impact.
  - Runbooks updated with current bootstrap steps.
- **Key Tasks**
  - Define playbook covering: signing key rotation under load, Redis limiter outage, Kafka/audit backlog catch-up.
  - Schedule first drill with security + SRE ownership and success metrics.
  - Automate helpers/scripts to trigger scenarios (e.g., toggle Redis endpoint, queue large audit batch).
  - Capture metrics before/after, document mitigation timeline, and log follow-up actions.
  - Store drill results in shared location (Confluence/Jira) linked from `docs/security/README.md`.
- **Acceptance Criteria**
  - Drill report created with timestamps, responsible engineers, and remediation notes.
  - Alerts/metrics observed match expectations, with MTTR recorded for each scenario.
  - Improvement items tracked for next iteration.
  - Recurring calendar event established (quarterly) with named owners.

## 4. Gateway Limiter Production Cutover
**Goal**: Enable the Redis-backed rate limiter in production with tuned thresholds and alerting.

- **Dependencies**
  - Redis cluster sized and monitored; secrets promoted.
  - Gateway metrics & alerts available (see Wave 5 dashboard).
- **Key Tasks**
  - Finalise per-tenant RPM ceilings (`GATEWAY_RATE_LIMIT_RPM`) and document rationale.
  - Enable limiter feature flag in staging, run synthetic load tests to validate acceptance traffic & block thresholds.
  - Verify Prometheus metrics `gateway_rate_limit_checks_total` / `gateway_rate_limit_rejections_total` and alert behaviour.
  - Update customer/partner comms if any high-volume tenants require whitelisting.
  - Promote config to production via IaC; monitor closely for first 48 hours.
  - Add rollback/override procedures to runbooks.
- **Acceptance Criteria**
  - Limiter enabled in production with dashboards confirming steady-state behaviour.
  - Alert tests fire to PagerDuty and resolve automatically after conditions clear.
  - No critical partner traffic blocked unexpectedly (validated via logs/support tickets).
  - Runbook updated with tuning + emergency override instructions.
