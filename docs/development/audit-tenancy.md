# Tenancy & Audit Unification (Initial Extraction)

## Overview

This iteration introduced a new `common-audit` crate and integrated it into `product-service` and `order-service` to begin converging on a unified tenancy + audit approach.

### Goals Achieved

- Central actor extraction helper (`extract_actor_from_headers`) uses JWT claim raw fields plus override headers (`X-User-*`).
- `AuditProducer` abstraction wraps a Kafka `FutureProducer` with a structured JSON envelope.
- Product + Order create/update (product: create/update/delete; order: create) now emit audit events conditionally (only if `audit_producer` is configured).
- Temporary DB-specific audit logging in `product-service` retained for backward compatibility.
- Added placeholder `/audit/events` route in both services returning `501 Not Implemented` (scaffold for centralized search API).

### Event Schema (JSON)

```json
{
  "id": "<uuid>",
  "tenant_id": "<uuid>",
  "actor": { "id": "<uuid>", "name": "...", "email": "..." },
  "entity_type": "product|order|...",
  "entity_id": "<uuid|null>",
  "action": "created|updated|deleted|...",
  "occurred_at": "RFC3339",
  "changes": {},
  "meta": { "source": "product-service" }
}
```

## Wiring a Real Audit Producer

Set an env var for topic name (recommended):

`AUDIT_TOPIC=audit.events`

Provide a Kafka producer (reuse existing bootstrap config) and initialize:

```rust
let audit = common_audit::AuditProducer::new(Some(kafka_producer.clone()), AuditProducerConfig { topic: audit_topic });
state.audit_producer = Some(audit);
```

(You can move this into each service until a central bootstrap helper is extracted.)

## Migration Notes

- Gradually replace ad-hoc per-entity audit tables once downstream consumers (e.g. analytics / admin portal) switch to Kafka-derived storage or a dedicated audit index.
- Keep DB logs during transition for forensic gap coverage.

## Next Steps (Recommended)

1. Introduce `TenantContext` lightweight extractor (wrapper around claims + header validation) to remove repeated `tenant_id_from_request` usages (or re-export existing guard in a narrower API surface).
2. Add standard error codes for audit emission failures (currently warn + continue).
3. Implement `/audit/events` backed by a read model (e.g. Postgres table populated by a single consumer service or a compacted Kafka topic query via Materialize/ksqlDB - future evaluation).
4. Add integration tests verifying audit payload shape for product/order happy paths.

## Offline Order Replay Validation Plan (Preview)

Purpose: ensure offline-captured orders (POS) reconcile with authoritative inventory + pricing before finalizing.

### Endpoint

`POST /orders/replay/validate`

### Request Shape

```json
{
  "client_order": {
    "id": "<uuid>",
    "created_at": "RFC3339",
    "items": [ {"product_id":"<uuid>","quantity":2,"unit_price":"12.34","line_total":"24.68"} ],
    "total":"24.68",
    "payment_method":"cash",
    "offline_idempotency_key": "<string>"
  },
  "client_version": "pos-app/1.4.2",
  "submitted_at": "RFC3339"
}
```

### Response (No Conflicts)

```json
{
  "status":"ok",
  "recalculated_total":"24.68",
  "normalized_items":[],
  "conflicts":[]
}
```

### Response (Conflicts)

```json
{
  "status":"conflict",
  "recalculated_total":"25.10",
  "conflicts":[
    {"type":"PRICE_MISMATCH","product_id":"...","client":"12.34","server":"12.55"},
    {"type":"OUT_OF_STOCK","product_id":"...","requested":3,"available":1}
  ]
}
```

### Conflict Codes (Initial Set)

- `PRICE_MISMATCH`
- `OUT_OF_STOCK`
- `PRODUCT_INACTIVE`
- `RESERVATION_EXPIRED` (if prior hold invalid)
- `UNKNOWN_PRODUCT`

### Flow

1. Validate tenant via shared guard.
2. Fetch authoritative product rows (price, active flag) in one batch.
3. Query current available inventory per product/location (optionally simulate reservation).
4. Compute recalculated total (BigDecimal -> Money normalization).
5. Aggregate conflicts; if empty return `ok`, else `conflict`.
6. (Future) Optionally persist a replay validation log row for analytics.

### Future Enhancements

- Provide server-suggested adjusted items (quantity or price corrections) to pre-fill resolution modal.
- Add `override_allowed` if roles (e.g. manager) can force acceptance with audit event.
- Tie into audit producer: `order.validation_attempt` events with conflict summary.

### Metrics to Add

- `offline_validation_attempts_total{outcome="ok|conflict"}`
- `offline_validation_conflicts_total{type=...}`

## Open Questions

- Should price rounding policy differences be surfaced as a distinct conflict type? (e.g. `ROUNDING_DELTA`)
- Do we pre-reserve inventory during validation or delay until final commit after user resolves conflicts?

---

(Initial draft – iterate as we implement.)
