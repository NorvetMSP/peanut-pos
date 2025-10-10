# Refund/Reversal Passthrough (P6-03)

Goal

- Map Order returns/exchanges to Payment reversals using original tender reference to ensure accurate refunds across providers.

Contracts

- Input from order-service to payment-service (JSON):
  - id: string (payment_intent id) OR providerRef: string (gateway transaction id)
  - amountMinor: number (optional; default full captured amount if omitted)
  - currency: string (ISO 4217)
  - reason: string (optional)
  - metadata: object (optional)
- Endpoint: POST /payment_intents/refund
- Response: { id, state: "refunded" | "failed", providerRef?, errorReason? }

Linkage

- Store either `intent_id` or `provider_ref` on the order’s tender sub-records at capture time.
- On return/exchange, compute refund total across returned lines and call refund once per tender allocation.

Error modes

- 400 invalid_request (mismatched currency/amount, missing linkage)
- 404 not_found (unknown intent)
- 409 conflict (already refunded greater than captured)
- 424 provider_failed (upstream decline)
- 429 throttled (retry with backoff)

Sequencing

- For exchanges creating a new order:
  1) Calculate refundable amount from return items
  2) Call payment-service refund with intent/providerRef
  3) Update order status: PARTIAL_REFUNDED or REFUNDED
  4) Emit audit events: refund.requested, refund.succeeded|failed

Idempotency

- Supply idempotencyKey per refund request (derived from return_id + tender_id + attempt)
- payment-service enforces unique idempotencyKey to prevent duplicate refunds

Audit

- Include tenant_id, order_id, intent_id/provider_ref, amountMinor, reason

Observability

- Counters: refunds_total{status}
- Histogram: refund_latency_seconds
- Labels: tenant_id, provider, result

Notes

- Partial refunds: support multiple calls up to captured amount
- Multi-tender: iterate across tender allocations; map per-intent
- Rounding: use integer minor units throughout
