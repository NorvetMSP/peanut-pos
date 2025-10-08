# Monitoring quickstart

This folder contains starter Grafana dashboards for NovaPOS.

Dashboards

- grafana/dashboards/checkout-kpis.json — checkout KPIs (latency p50/p95, tap count, errors)
- grafana/dashboards/kafka-lag.json — Kafka consumer group lag panels

Prometheus

- Ensure your services expose Prometheus metrics at /metrics
- Scrape example (replace targets):

```yaml
scrape_configs:
  - job_name: 'novapos-services'
    static_configs:
      - targets: ['localhost:8081','localhost:8082','localhost:8083','localhost:8084','localhost:8085','localhost:8086','localhost:8087','localhost:8088','localhost:8089']
```

Kafka exporter (optional)

- Deploy kafka_exporter and point to your brokers; expose metrics to Prometheus

Grafana

- Import the JSON files in grafana/dashboards
- Set the Prometheus datasource name to "Prometheus" or adjust in dashboards

Notes

- KPI panels expect metrics like `checkout_latency_seconds_bucket`, `checkout_tap_count`, `checkout_errors_total`
- Kafka lag panels expect `kafka_consumergroup_lag`

Alerting

- Add rule files from `monitoring/prometheus/alerts/novapos-rules.yaml` to your Prometheus `rule_files` and reload.

Service flags and new metrics

- order-service
  - Metrics: `outbox_published_total{topic}`, `outbox_publish_failures_total{topic}`, `outbox_publish_retries_total{topic}`, `outbox_backlog`
  - Flags: `OUTBOX_WORKER=1` to run publisher; `ORDER_OUTBOX_MODE=1` to enqueue to outbox instead of direct Kafka
- inventory-service
  - Inbox de-dup enabled by default; disable with `INVENTORY_INBOX_DEDUP=0`
- loyalty-service
  - Inbox de-dup enabled by default; disable with `LOYALTY_INBOX_DEDUP=0`
- analytics-service
  - Inbox de-dup enabled by default; disable with `ANALYTICS_INBOX_DEDUP=0`
