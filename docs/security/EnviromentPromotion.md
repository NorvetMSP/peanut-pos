Staging Rollout Workflow

Promote Secrets

Follow docs/security/secret-promotion-guide.md to export from local Vault and publish into the staging secret manager.
Required key sets: secret/novapos/auth, secret/novapos/integration-gateway; include the refresh-cookie env vars (AUTH_REFRESH_COOKIE_*) with staging domain/secure=true.
Update Helm/ECS/task definitions so staging services read the new secret references.
Deploy Updated Services

Deploy auth-service with the new /session + /logout endpoints and refresh-cookie config.
Ensure frontends (admin portal + POS) push the latest build that relies on cookie sessions.
Confirm Redis/Kafka are reachable; run database migrations if pending (auth refresh tokens, etc.).
Monitoring & Alerting Bootstrap

Start Prometheus/Grafana stack or point staging instance at repo configs.
Copy monitoring/prometheus/prometheus.yml and the alerts/ directory; verify the /etc/prometheus/alerts/security-wave5.rules.yml rules load (check http://staging-prom:9090/rules).
Import monitoring/grafana/provisioning/dashboards/wave5.json into Grafana; retarget datasource to staging Prometheus.
Wire Alertmanager/PagerDuty as in docs/security/README.md (Grafana and PagerDuty Wiring Checklist). Trigger test alerts.
Smoke Tests & Runbooks

Run the MFA/login telemetry script in docs/security/README.md to exercise invalid password, missing MFA, bad MFA, success, and refresh flow.
Verify /metrics endpoints for auth-service and integration-gateway expose the expected counters.
Update runbook references if staging-specific dashboards or alert IDs differ.
Follow-On Planning

Create staging tickets per docs/security/wave5-follow-on-tickets.md; ensure staging environment is ready for audit-service, JWKS adoption, DR drills, and limiter testing.
Record results in the Wave 5 Jira epic.
Production Rollout Workflow

Secret Promotion & Verification

Repeat the secret promotion process with production vault/secret manager.
Double-check cookie settings (Secure=true, SameSite=None if cross-origin) and production domains.
Controlled Deployment

Deploy auth-service and frontends during a maintenance window or low-traffic period.
Enable Prometheus scrape targets and Grafana dashboards pointing to production endpoints.
Flip rate limiter feature flags cautiously (keep in monitor-only mode until ready).
Validation & Alert Drills

Execute the MFA/login smoke script against production (use service accounts) to ensure telemetry flows.
Trigger synthetic alerts (e.g., manually push gateway_rate_limit_rejections_total above threshold) to verify PagerDuty escalation pathways.
Capture evidence (screenshots, logs) for compliance.
Documentation & Post-Rollout

Update docs/security/README.md with any production-specific nuances discovered.
Confirm docs/security/security-hardening-rust-addendum.md checklist items are marked complete for production.
Store production run artifacts (alert IDs, Grafana links) referenced by auditors.
Queue Follow-On Work

Move the tickets from docs/security/wave5-follow-on-tickets.md into the production backlog; ensure each has owners and timelines (audit-service deployment, fleet JWKS, DR drills, limiter cutover).
Schedule the first DR drill and create recurring calendar invites.
By following these staging → production workflows, you’ll use every relevant doc under docs/security/: the README for runbooks, the addendum for status and references, the secret promotion guide for vault actions, and the new ticket backlog for Wave 5 follow-on tasks.