# NOVAPOS Security Runbooks

Wave 5 hardened auth-service and integration-gateway now publish security telemetry. Use these runbooks when bringing up new environments, responding to alerts, or running scheduled smoke tests.

## Monitoring Bootstrap Prerequisite
Prometheus and Grafana are not yet deployed. Stand up the monitoring stack described in `prometheus-grafana-bootstrap.md` before attempting to wire dashboards or alert rules.
## Vault Bootstrap (All Environments)
1. Ensure Vault is reachable. For local development use `http://127.0.0.1:8200`; for staging/production use the platform endpoint.
2. Export required variables before running the CLI:
   ```powershell
   $env:VAULT_ADDR = 'http://127.0.0.1:8200'
   $env:VAULT_TOKEN = 'root'
   ```
3. Seed the secret mounts when standing up a new environment:
   ```powershell
   vault kv put secret/novapos/auth \
     KAFKA_BOOTSTRAP='kafka:9092' \
     REDIS_URL='redis://redis:6379/0' \
     SECURITY_MFA_ACTIVITY_TOPIC='security.mfa.activity'

   vault kv put secret/novapos/integration-gateway \
     SECURITY_ALERT_TOPIC='security.alerts.v1'
   ```
4. Retrieve a secret to confirm connectivity:
   ```powershell
   vault kv get secret/novapos/auth
   ```
5. Follow the [Wave 5 Secret Promotion Guide](secret-promotion-guide.md) to mirror these values into staging and production managers.

## Auth Telemetry & MFA Smoke Test
Use this script after every deployment or when investigating auth anomalies.

1. Export the environment variables (local example):
   ```powershell
   $env:KAFKA_BOOTSTRAP = 'localhost:9092'
   $env:SECURITY_MFA_ACTIVITY_TOPIC = 'security.mfa.activity'
   $env:SECURITY_SUSPICIOUS_WEBHOOK_URL = ''
   $env:SECURITY_SUSPICIOUS_WEBHOOK_BEARER = ''
   ```
2. Start dependencies and auth-service:
   ```powershell
   docker compose up -d postgres kafka redis
   docker compose up -d --build auth-service
   ```
3. (Optional) enable MFA for the admin seed user:
   ```powershell
   docker compose exec -T postgres psql -U novapos -d novapos \
     -c "UPDATE users SET mfa_secret='JBSWY3DPEHPK3PXP', mfa_enrolled_at=NOW(), mfa_failed_attempts=0 WHERE email='admin@novapos.local';"
   ```
4. Generate a current TOTP code when prompted:
   ```powershell
   python scripts/totp.py 'JBSWY3DPEHPK3PXP'
   ```
   (If `scripts/totp.py` is not available, use the snippet in the addendum.)
5. Exercise the login flow to hit each telemetry path:
   ```powershell
   # invalid password
   Invoke-WebRequest -UseBasicParsing -Uri 'http://localhost:8085/login' -Method POST -Headers @{ 'Content-Type' = 'application/json'; 'X-Tenant-ID' = $tenant } -Body '{"email":"admin@novapos.local","password":"wrong"}'

   # missing MFA
   Invoke-WebRequest -UseBasicParsing -Uri 'http://localhost:8085/login' -Method POST -Headers @{ 'Content-Type' = 'application/json'; 'X-Tenant-ID' = $tenant } -Body '{"email":"admin@novapos.local","password":"admin123"}'

   # bad MFA
   Invoke-WebRequest -UseBasicParsing -Uri 'http://localhost:8085/login' -Method POST -Headers @{ 'Content-Type' = 'application/json'; 'X-Tenant-ID' = $tenant } -Body '{"email":"admin@novapos.local","password":"admin123","mfaCode":"000000"}'

   # success (prompt for $goodCode)
   $goodCode = Read-Host 'Enter current TOTP'
   Invoke-WebRequest -UseBasicParsing -Uri 'http://localhost:8085/login' -Method POST -Headers @{ 'Content-Type' = 'application/json'; 'X-Tenant-ID' = $tenant } -Body '{"email":"admin@novapos.local","password":"admin123","mfaCode":"' + $goodCode + '"}'
   ```
6. Verify counters:
   ```powershell
   curl http://localhost:8085/metrics | findstr auth_login_attempts_total
   curl http://localhost:8085/metrics | findstr auth_mfa_events_total
   ```
7. Record the results in the deployment log.

## Integration Gateway Rate Limit Response
1. Bring the gateway online with Redis/Kafka:
   ```powershell
   $env:SECURITY_ALERT_TOPIC = 'security.alerts.v1'
   $env:SECURITY_ALERT_WEBHOOK_URL = ''
   $env:SECURITY_ALERT_WEBHOOK_BEARER = ''
   $env:REDIS_URL = 'redis://localhost:6379/0'
   docker compose up -d redis postgres kafka
   docker compose up -d --build integration-gateway
   ```
2. Acquire an auth token from auth-service (reuse the smoke script above).
3. Drive traffic to sample both allowed and rejected paths:
   ```powershell
   1..75 | ForEach-Object {
     Invoke-WebRequest -UseBasicParsing -Uri 'http://localhost:8083/api/tenant/ping' -Headers @{ 'Authorization' = "Bearer $token" }
   }
   ```
4. Inspect gateway metrics:
   ```powershell
   curl http://localhost:8083/metrics | findstr gateway_rate_limit_checks_total
   curl http://localhost:8083/metrics | findstr gateway_rate_limit_rejections_total
   ```
5. When alerts fire, capture the PagerDuty incident ID and annotate with tenant, rate limit settings, and any Redis health observations.

## Grafana and PagerDuty Wiring Checklist
> Prerequisite: complete the monitoring bootstrap in `prometheus-grafana-bootstrap.md` so Prometheus and Grafana are available.
1. Add the following Prometheus scrape targets if they are not already present: auth-service `/metrics`, integration-gateway `/metrics`.
2. Dashboard panels (PromQL examples):
   ```promql
   sum by (outcome) (rate(auth_login_attempts_total[5m]))
   sum by (outcome, route) (rate(auth_mfa_events_total[5m]))
   sum by (outcome) (rate(gateway_rate_limit_checks_total[5m]))
   sum(rate(gateway_rate_limit_rejections_total[5m]))
   ```
3. PagerDuty alert thresholds:
   - `auth_login_attempts_total{outcome="invalid_credentials"}` spike: trigger when the 5m rate exceeds 50 per tenant.
   - `auth_mfa_events_total{outcome="mfa_invalid"}` spike: trigger when the 5m rate exceeds 10 per tenant.
   - `gateway_rate_limit_rejections_total`: trigger when the 1m rate exceeds 5 for any tenant.
4. Link this document in the alert runbooks section of PagerDuty so on-call engineers can jump straight to the diagnostics.

## Change Management
- When secrets or thresholds change, update both this README and the [Security Hardening Addendum](security-hardening-rust-addendum.md).
- Document completed smoke tests in the Wave 5 Jira epic to preserve the audit trail.

