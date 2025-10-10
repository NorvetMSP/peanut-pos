# Checkout KPI Instrumentation (P0-04)

Goal

- Measure and visualize end-to-end checkout performance and effort.

Primary metrics

- Metric: `checkout_latency_seconds` (histogram)
  - Labels: tenant_id, store_id, terminal_id
  - Suggested buckets (s): 0.25, 0.5, 1, 1.5, 2, 3, 5, 8
  - SLO: p95 < 2s
- Metric: `tap_count_total` (counter)
  - Labels: tenant_id, store_id, terminal_id
  - Definition: number of UI taps/clicks from cart ready → order submitted; POS emits
  - SLO: p95 < 5 taps

Trace model (W3C propagation)

- Root span: `pos.checkout`
  - child: `order.submit`
    - child: `inventory.reserve`
    - child: `payment.intent` → `payment.capture`
- Required attributes on spans: tenant_id, store_id, terminal_id, order_id

Surfacing

- Grafana panels: p50/p95 latency per tenant/store; tap_count distribution
- Alerts:
  - `checkout_latency_seconds{quantile="0.95"} > 2` for 5m
  - `increase(tap_count_total[10m]) > 5` (per session approximation via panel drilldown)

Implementation outline

- POS: capture start/end times and tap counts; include traceparent header on server calls
- Services: accept/propagate trace headers; record handler/span timings; expose metrics on `/metrics`
- Integration: prefer OpenTelemetry for spans; use Prometheus client for metrics

Next steps today

- Add no-op metric names in services and a TODO tracer in POS; no runtime behavior change
