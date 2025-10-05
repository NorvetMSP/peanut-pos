# RFC: Exchanges Backend API (MVP)

Prepared: 2025-10-05

Status: Draft

Owners: order-service

## Summary

Support exchanges by referencing an original order, returning selected items, and creating a new order for replacement items in a single orchestrated API call. Keep data model minimal: link the new order to the original via a nullable column (`exchange_of_order_id`) and reuse existing refunds and order creation logic.

## Goals

- A single endpoint to perform a basic exchange: return some items from an existing order and purchase new items.
- Correct inventory movements: restock returned items; decrement for new items.
- Compute net delta: refund owed to customer or amount due from customer.
- Persist a link between the new order and the original order.
- Emit events for audit: refund, new order, exchange summary.

## Non-Goals (MVP)

- Complex policy (who can authorize exchanges) beyond role gating.
- Splitting refunds across multiple original payment methods.
- Multiple new orders per exchange (we create exactly one new order per call).
- Store credit issuance; gift receipts; tax jurisdiction changes.

## API Design

Endpoint:

- POST `/orders/{original_order_id}/exchange`

Request headers (existing):

- `Authorization: Bearer <jwt>` (must include tenant `tid`)
- `X-Tenant-ID: <uuid>` (must match JWT `tid`)
- `X-Roles: Manager,Admin` (MVP: exchanges require Manager/Admin)

Request body:

```json
{
  "return_items": [
    { "order_item_id": "<uuid>", "qty": 1 },
    { "order_item_id": "<uuid>", "qty": 2 }
  ],
  "new_items": [
    { "sku": "ABC123", "qty": 1 },
    { "sku": "XYZ789", "qty": 1 }
  ],
  "discount_percent_bp": 0,
  "payment": { "method": "card" },
  "cashier_id": "<uuid>",
  "idempotency_key": "optional-unique-key"
}
```

Notes:

- `return_items` references specific order items from the original order; quantity must not exceed remaining refundable quantity (original qty minus prior refunds).
- `new_items` are specified by SKU (or product_id if needed; SKU preferred for POS).
- `payment` is optional if net delta is a refund to customer (we proceed with a refund using the original order's primary method if feasible; fallback to cash in MVP).
- `discount_percent_bp` applies to the new order (same computation rules as create order).

Response example (net amount due):

```json
{
  "original_order_id": "...",
  "exchange_order_id": "...", 
  "refund": {
    "refund_id": "...",
    "refunded_cents": 2500
  },
  "new_order": {
    "order_id": "...",
    "totals": {
      "subtotal_cents": 3000,
      "discount_cents": 0,
      "tax_cents": 247,
      "total_cents": 3247
    }
  },
  "net_delta_cents": 747,          
  "net_direction": "collect",     
  "payment": { "method": "card", "status": "captured", "amount_cents": 747 },
  "receipt_text": "... optional combined or new order receipt ..."
}
```

Response example (net refund):

```json
{
  "original_order_id": "...",
  "exchange_order_id": "...",
  "refund": {
    "refund_id": "...",
    "refunded_cents": 1500
  },
  "new_order": {
    "order_id": "...",
    "totals": { "total_cents": 950 }
  },
  "net_delta_cents": -550,          
  "net_direction": "refund",
  "refund_to_customer": { "method": "original|cash", "status": "captured", "amount_cents": 550 }
}
```

Idempotency:

- Requests may include `idempotency_key`. If repeated, we return the prior result.

## Data Model Changes

- Alter `orders` table to add a nullable link to an original order when this order is created as part of an exchange.

```sql
ALTER TABLE orders
ADD COLUMN exchange_of_order_id UUID NULL REFERENCES orders(id) ON DELETE SET NULL;
CREATE INDEX idx_orders_exchange_of ON orders(exchange_of_order_id);
```

- No new tables required. Existing `payments` and `refunds/returns` flows are reused.

## Orchestration Flow (Happy Path)

1. Validate auth/tenant/roles (Manager/Admin).
2. Load original order with items and prior refunds; ensure status is `paid` (or equivalent complete).
3. Validate `return_items` quantities against refundable balance.
4. Compute refund for `return_items` using existing partial refund computation (proportional discount and tax rules consistent with MVP).
5. Apply inventory restock for returned items (via inventory-service) once refund is persisted.
6. Create new order for `new_items` with `exchange_of_order_id = original_order_id`; compute totals and apply discount.
7. Reserve/decrement inventory for new items via inventory-service. If reservation fails, roll back the new order and the refund.
8. Compute net delta: `new_order.total_cents - refunded_cents`. If > 0, collect from customer using `payment.method`. If <= 0, issue the difference as a refund (MVP: same method as original primary payment if feasible; else cash fallback).
9. Persist payments/refunds and finalize both records.
10. Emit events: `pos.refund`, `pos.order` (new order), and `pos.exchange` summary event with linkage IDs.

## Authorization

- MVP: `Manager` or `Admin` role required.
- Future: configurable policy to allow `Cashier` for exchanges below a threshold or with manager override.

## Inventory Interactions

- Returned items: increment on-hand (restock) when refund finalizes.
- New items: decrement on-hand when new order is created/captured, consistent with current order flow.
- Both actions must be tenant-scoped and idempotent.

## Payments and Refunds

- Reuse existing refund endpoint logic internally for `return_items` (no public multi-step required).
- Reuse existing payment capture path for the net amount to collect.
- For net refunds, use original primary payment method where possible; otherwise, fall back to cash in MVP.

## Errors and Edge Cases

- 400 `invalid_request`: empty `return_items` and `new_items` simultaneously, or invalid payload fields.
- 404 `order_not_found`: original order not located for tenant.
- 409 `refundable_qty_exceeded`: attempted refund qty exceeds remaining refundable balance.
- 409 `inventory_unavailable`: unable to reserve new items.
- 422 `order_not_completed`: original order not in a refundable state.
- 403 `forbidden`: insufficient role for exchange.

## Events (MVP)

- `pos.refund`: includes original order, items, and amount.
- `pos.order`: emitted for the new order as today.
- `pos.exchange`: summary linking original and exchange orders with net delta.

## Observability

- Log correlation IDs across refund and new order creation.
- Metrics: exchange attempts, successes, failures by reason.

## Rollout Plan

1. Add DB column + index.
2. Implement endpoint behind a feature flag `exchanges` (enabled in dev by default).
3. Add integration tests (net refund, net collect, qty exceed, inventory fail).
4. Update admin reports later to include exchange linkage where useful.

## Open Questions

- Refund method selection for split tenders: defer to future.
- Authorization: should Cashier be allowed for even exchanges (net zero)? Future policy.
- Receipt output: combined vs. separate; MVP returns the new order receipt and includes refund summary.

## Appendix: Sample curl

```bash
# EXAMPLE ONLY (PowerShell adaptation needed):
curl -X POST \
  -H "Authorization: Bearer <jwt>" \
  -H "X-Tenant-ID: <tid>" \
  -H "X-Roles: Manager" \
  -H "Content-Type: application/json" \
  -d '{
    "return_items": [{"order_item_id":"<uuid>","qty":1}],
    "new_items": [{"sku":"ABC123","qty":1}],
    "payment": {"method":"card"},
    "cashier_id": "<uuid>"
  }' \
  https://order-service.local/orders/<original_order_id>/exchange
```
