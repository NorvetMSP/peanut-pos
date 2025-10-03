# Monitoring & Observability (Stub)

This directory houses Prometheus alert rules and Grafana dashboard stubs for the NovaPOS platform.

## Components

- `prometheus/rules/integration-gateway-alerts.yaml` – Alerting rules for integration-gateway latency, backpressure, rate limiter usage, and error code hygiene.
- `grafana/dashboards/integration-gateway-overview.json` – Gateway overview dashboard panels (latency quantiles, channel depth, rate window usage, capability denials, denial rate %, error code saturation).

## Loading Prometheus Rules (Kubernetes)

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: integration-gateway-prom-rules
  labels:
    role: prometheus-rule
    prometheus: main
    app.kubernetes.io/name: novapos
    app.kubernetes.io/component: integration-gateway
data:
  integration-gateway-alerts.yaml: |
    {{- (.Files.Get "monitoring/prometheus/rules/integration-gateway-alerts.yaml") | nindent 4 -}}
```

If not using Helm templating, inline the file contents under the `data` key directly.

Prometheus Operator example (CRD):

```yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: integration-gateway-rules
  labels:
    prometheus: main
    role: alert-rules
spec:
  groups:
  - name: integration-gateway.latency-and-backpressure
    rules: # Paste rules from integration-gateway-alerts.yaml here
```

## Loading Grafana Dashboard

Using a ConfigMap (side-loaded by provisioning):

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: grafana-dashboard-integration-gateway
  labels:
    grafana_dashboard: "1"
    app.kubernetes.io/name: novapos
    app.kubernetes.io/component: integration-gateway
data:
  integration-gateway-overview.json: |
    {{- (.Files.Get "monitoring/grafana/dashboards/integration-gateway-overview.json") | nindent 4 -}}
```

Place in a directory referenced by Grafana's provisioning `dashboards.yaml` provider.

## Metric Reference (Gateway)

| Metric | Type | Description |
|--------|------|-------------|
| gateway_rate_limiter_decision_seconds | histogram | Rate limiter decision latency (seconds) |
| gateway_rate_limit_checks_total | counter | Total rate limit checks |
| gateway_rate_limit_rejections_total | counter | Total rate limit rejections |
| gateway_rate_window_usage | gauge | Current count in active rate window (last key) |
| gateway_rate_limit_rpm_target | gauge | Configured per-identity RPM limit target |
| gateway_channel_depth | gauge | Internal queue depth (synthetic/dev) |
| gateway_channel_capacity | gauge | Queue capacity |
| gateway_channel_high_water | gauge | Highest observed depth |
| capability_denials_total | counter | Authorization denials per capability |
| capability_checks_total | counter | Allow + deny counts for capabilities |
| http_error_code_saturation | gauge | Percentage of error code guard usage |
| http_error_code_overflow_total | counter | Overflow occurrences (should stay 0) |

## Dev Metrics Demo

Synthetic queue filling is active only when:
 
- Build is debug (`cfg(debug_assertions)`) OR
- Env var `GATEWAY_DEV_METRICS_DEMO=1` is set at process start

Production deployments should not set the env var; metrics should reflect real workload.

## Alert Thresholds Summary

| Area | Warning | Critical |
|------|---------|----------|
| Rate limiter p95 | >40ms 15m | >80ms 10m |
| Rate window usage | >85% 10m | (future) add >90% 5m |
| Backpressure depth | >70% 10m | (future) >85% 5m |
| Error code saturation | >70% 15m | >85% 10m |
| Error code overflow | n/a | any increase |
| Capability denial rate | >25% (panel-only now) | >40% (future alert) |

## Future Work

- Add capability denial rate alert once baseline established.
- Add backpressure percent calculation panel (depth / capacity).
- Extend dashboards with tenant dimension carefully (cardinality watch).
- Optionally export rate limiter window size to differentiate multiple policies.
