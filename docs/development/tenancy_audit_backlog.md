# Tenancy & Audit Unification Backlog

Source-of-truth tracker derived from MVP Gap Build doc and option pathways.

## Tracking Model

Each work item has an ID `TA-<Category><Sequence>`.

Categories:

- FND: Foundation / cleanup
- ROL: Rollout to additional services
- AUD: Audit platform (producer, consumer, search, retention, redaction)
- PERF: Performance / reliability
- POL: Policy / authorization evolution
- OPS: Observability / tooling
- DOC: Documentation
- FUT: Research / future evaluation

Status: Planned | In-Progress | Done | Blocked | Deferred

## RACI (Lightweight)

- Owner: primary engineer
- Approver: tech lead
- Consulted: security, data stakeholders
- Informed: frontend, analytics teams

## Dashboard Metrics (Planned)

- audit_emit_latency_p95 (ms)
- audit_queue_depth
- audit_events_ingested_total
- audit_events_redacted_total
- audit_coverage_ratio
- auth_regression_incidents (per sprint)

## Items

| ID | Title | Category | Status | Depends | Notes |
|----|-------|----------|--------|---------|-------|
| TA-FND-1 | Remove legacy role constants & helpers | FND | Done | — | Completed: product-service cleanup |
| TA-FND-2 | Add trace_id auto-generation in extractor | FND | Done | — | Implemented in SecurityCtxExtractor |
| TA-ROL-1 | Apply SecurityCtxExtractor to inventory-service | ROL | Planned | TA-FND-1 | Inventory mutating endpoints |
| TA-ROL-2 | Apply SecurityCtxExtractor to loyalty-service | ROL | Planned | TA-FND-1 | Align role mapping |
| TA-ROL-3 | Apply SecurityCtxExtractor to payment-service | ROL | Planned | TA-FND-1 | Pre-req payment intent model |
| TA-ROL-4 | Apply SecurityCtxExtractor to customer-service | ROL | Planned | TA-FND-1 | Remove per-handler tenant parsing |
| TA-ROL-5 | Apply SecurityCtxExtractor to integration-gateway | ROL | Planned | TA-FND-1 | Propagate trace & roles downstream |
| TA-FND-3 | Unified auth error JSON shape | FND | Planned | TA-FND-1 | {code,missing_role,trace_id} |
| TA-AUD-1 | Buffered AuditProducer (async channel) | AUD | Done | TA-FND-2 | Integrated in product & order services |
| TA-OPS-1 | Metrics: queue length & emit failures | OPS | Done | TA-AUD-1 | Prometheus /internal/metrics + JSON legacy endpoint (deprecate after dashboards) |
| TA-AUD-2 | Audit consumer + Postgres read model | AUD | Done | TA-AUD-1 | Kafka->PG service, lag & latency histogram, failed counter, last ingest timestamp, optional batching |
| TA-AUD-3 | /audit/events query endpoint | AUD | Done | TA-AUD-2 | Added filters, event_id cursor, severity normalization |
| TA-AUD-4 | Coverage scanner tool | AUD | Done | TA-AUD-1 | AST-based (syn) parser, per-service config, JSON report, Prometheus metrics file, CI 90% ratio gate |
| TA-AUD-5 | Retention policy (TTL purge job) | AUD | Done | TA-AUD-2 | Background purge, env AUDIT_RETENTION_DAYS (default 30), dry-run, metrics (deleted total, last run) |
| TA-AUD-6 | Redaction tagging + masking layer | AUD | Done | TA-AUD-2 | Configurable field paths env-driven, masking modes (off/log/enforce), metrics (redacted total, last timestamp) |
| TA-AUD-7 | Role-based redacted view | AUD | Done | TA-AUD-6 | Privileged (Admin) gating implemented; response-layer redaction with masking/removal variant and metrics + labeled counters. Reusable `redact_event_fields` helper & unit tests (mask/remove/noop) plus full HTTP integration test validating Support vs Admin + include_redacted masking path. |
| TA-PERF-1 | Outbox pattern for audit durability | PERF | Planned | TA-AUD-2 | Optional fallback when Kafka down |
| TA-PERF-2 | Backpressure metrics & alerts | PERF | Planned | TA-AUD-1 | Alert on queue saturation |
| TA-POL-1 | Expanded role model (Cashier vs Support) | POL | Planned | TA-ROL-* | Enum refinement |
| TA-POL-2 | Policy engine evaluation spike | POL | Planned | TA-POL-1 | Cedar/Oso assessment |
| TA-DOC-1 | Developer guide: emitting audit event | DOC | Planned | TA-AUD-1 | CONTRIBUTING snippet |
| TA-DOC-2 | Architecture doc: audit pipeline phases | DOC | Planned | TA-AUD-2 | Sequence & flow diagrams |
| TA-OPS-2 | Schema compliance linter / macro | OPS | Planned | TA-AUD-1 | Build-time validation |
| TA-OPS-3 | Prometheus client metrics migration | OPS | Done | TA-OPS-1 | Migrated to `prometheus` crate: registered buffer gauges/counters + redaction counters (labelled). Added metrics endpoint integration test. Future: dedupe double-count risk & cardinality guard. |
| TA-AUD-8 | Multi-sink support (Kafka + Search) | AUD | Deferred | TA-AUD-3 | Latency-driven follow-on |
| TA-FUT-1 | Search backend evaluation | FUT | Planned | TA-AUD-2 | PG vs ES vs ClickHouse bench |
| TA-FUT-2 | Tenant isolation study (RLS vs shared) | FUT | Planned | — | Decision doc |

## Initial Execution Order (Sprint 1–2)

1. TA-FND-1, TA-FND-2
2. TA-AUD-1
3. TA-OPS-1 & TA-AUD-4 (observability + coverage)
4. TA-AUD-2 then TA-AUD-3
5. Begin TA-ROL-1 / TA-ROL-2 post buffered stability

## Definition of Done

- Code merged + tests
- Docs updated (if user/dev facing)
- Metrics added (if observability-related)
- Change log entry in `MVP_GAP_BUILD.md` for Partial→Done transitions

## Coverage Scanner Concept (TA-AUD-4)

Heuristic: scan handler source for verbs (create|update|delete|refund|void|adjust). Flag absence of `audit_producer.emit(`. Allow `// audit:ignore` suppression.

## Risk Mitigation Highlights

- Channel overflow: drop counter + sample warn log
- Consumer lag: expose `audit_consumer_lag_seconds`
- Redaction regression: table-driven before/after tests

## Open Questions

- Cross-tenant (super-admin) search needed in MVP? (Assumed: No)
- Default retention period (Assumed: 30 days) configurable per tenant?
- Severity taxonomy expansion (Warn, Error, Security) now or later?

## Update Cadence

Weekly backlog status refresh; per PR: link item ID + update change log.

---
(Generated baseline backlog; refine IDs or sequencing as needed.)
