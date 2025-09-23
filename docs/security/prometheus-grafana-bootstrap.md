# Prometheus & Grafana Bootstrap

## Purpose
Auth-service and integration-gateway expose `/metrics`, but the monitoring stack is not yet deployed. This guide walks through standing up Prometheus and Grafana for local validation and then promoting the stack to staging/production.

## Overview
1. Provision Prometheus to scrape the services.
2. Provision Grafana to visualise the new counters.
3. Persist configuration so the deployment mirrors across environments.

## Local Quick Start (Docker Compose)
`docker-compose.yml` now includes the monitoring services; review or adjust the block below if you customise ports or networks:

```yaml
prometheus:
  image: prom/prometheus:v2.54.0
  container_name: novapos-prometheus
  volumes:
    - ./monitoring/prometheus/prometheus.yml:/etc/prometheus/prometheus.yml:ro
  command:
    - '--config.file=/etc/prometheus/prometheus.yml'
  ports:
    - '9090:9090'
  networks:
    - novanet

grafana:
  image: grafana/grafana:11.1.3
  container_name: novapos-grafana
  environment:
    - GF_SECURITY_ADMIN_USER=admin
    - GF_SECURITY_ADMIN_PASSWORD=admin
  volumes:
    - ./monitoring/grafana/provisioning:/etc/grafana/provisioning
  ports:
    - '3002:3000'
  depends_on:
    - prometheus
  networks:
    - novanet
```
> Host port 3002 avoids collisions with the POS (3000) and admin portal (3001) frontends; adjust if those services are disabled.


`monitoring/prometheus/prometheus.yml` is checked in with the following defaults (tweak targets as needed):

```yaml
global:
  scrape_interval: 15s
  evaluation_interval: 30s

scrape_configs:
  - job_name: 'auth-service'
    metrics_path: /metrics
    static_configs:
      - targets: ['auth-service:8085']

  - job_name: 'integration-gateway'
    metrics_path: /metrics
    static_configs:
      - targets: ['integration-gateway:8083']
```

> Adjust hostnames/ports to match your compose network. Add additional jobs for other services as they adopt `/metrics`.

Grafana provisioning files are also tracked in-repo; update them if your environment needs different defaults:

`monitoring/grafana/provisioning/datasources/datasource.yaml`
```yaml
apiVersion: 1
datasources:
  - name: Prometheus
    type: prometheus
    access: proxy
    url: http://prometheus:9090
    isDefault: true
```

`monitoring/grafana/provisioning/dashboards/dashboard.yaml`
```yaml
apiVersion: 1
disableDeletion: false
providers:
  - name: 'Wave 5 Dashboards'
    orgId: 1
    folder: ''
    type: file
    updateIntervalSeconds: 10
    options:
      path: /etc/grafana/provisioning/dashboards
```

`monitoring/grafana/provisioning/dashboards/wave5.json` ships with an initial panel; expand it with exports from Grafana or the query list in the runbook.

Bring the stack up:
```powershell
docker compose up -d prometheus grafana
```

Visit Grafana at http://localhost:3002 (defaults: admin/admin). Add panels using the PromQL snippets in `docs/security/README.md`.

## Staging/Production Deployment
- **Kubernetes**: package the Prometheus chart (kube-prometheus-stack or Helm shared chart) with service monitors targeting the auth-service and integration-gateway pods/services.
- **VM/ECS**: deploy Prometheus and Grafana as systemd services or Fargate tasks referencing the same configuration layout as above.
- Capture configuration in IaC (Terraform, Helm, Pulumi) so environments stay in sync.

## Integration Tasks
1. Import the Wave 5 dashboard JSON into Grafana and tag it `Security`.
2. Configure Grafana alerting or export Prometheus alert rules to the main alertmanager stack.
3. Link Grafana dashboards and Prometheus queries in PagerDuty alert runbooks per `docs/security/README.md`.

## Validation Checklist
- Prometheus targets show `UP` for auth-service and integration-gateway.
- Metrics appear under `auth_login_attempts_total`, `auth_mfa_events_total`, `gateway_rate_limit_checks_total`, and `gateway_rate_limit_rejections_total`.
- Grafana dashboard panels render data within a few scrape intervals.
- PagerDuty alert test triggers include the Grafana links.

Record bootstrap completion in the Wave 5 epic to unblock the remaining monitoring tasks.



