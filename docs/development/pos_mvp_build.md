# POS MVP Build Plan

Focused objective: Deliver a cashier-facing flow (Scan / Add Item → View Cart + Totals → Take Payment → Persist Order → Emit Event → Receipt) in the smallest coherent vertical slice.

## 1. Functional Scope (MVP)

Must Have:

- Product lookup by SKU / barcode
- Cart operations (add, remove, change quantity)
- Tax calculation (flat rate per tax_code)
- Discount (single per cart OR single line discount) – choose cart % first
- Payment capture (mock card + cash) and change due logic
- Order persistence (order + items + payment rows)
- Receipt generation (structured JSON + plaintext rendering)
- Event emission (single `pos.order` event containing order + payment status)
- Basic role: Cashier (create orders), Manager (void/refund placeholder)

Should Have (next sprint):

- Returns / exchanges referencing original order
- Inventory decrement + low-stock event stub
- Loyalty points accrue (simple earn: e.g. 1 point per $1)
- End-of-day settlement (Z-report by tender type)
- Partial refunds

Deferred (intentionally not MVP):

- Advanced promotion engine
- Multi-currency / multi-jurisdiction tax matrix
- Policy engine (Cedar/Oso)
- Complex audit enrichment / redaction expansions
- Full offline sync implementation (initial design stub only)

## 2. Data Model

Tables (Postgres):

```sql
-- products
CREATE TABLE products (
  id UUID PRIMARY KEY,
  sku TEXT UNIQUE NOT NULL,
  name TEXT NOT NULL,
  price_cents INT NOT NULL,
  tax_code TEXT NOT NULL,
  active BOOLEAN NOT NULL DEFAULT TRUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- orders
CREATE TABLE orders (
  id UUID PRIMARY KEY,
  total_cents INT NOT NULL,
  subtotal_cents INT NOT NULL,
  tax_cents INT NOT NULL,
  discount_cents INT NOT NULL DEFAULT 0,
  status TEXT NOT NULL, -- 'created','paid','voided'
  cashier_id UUID NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- order_items
CREATE TABLE order_items (
  id UUID PRIMARY KEY,
  order_id UUID NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
  product_id UUID NOT NULL REFERENCES products(id),
  qty INT NOT NULL,
  unit_price_cents INT NOT NULL,
  tax_code TEXT NOT NULL,
  line_subtotal_cents INT NOT NULL,
  line_tax_cents INT NOT NULL,
  line_total_cents INT NOT NULL
);

-- payments
CREATE TABLE payments (
  id UUID PRIMARY KEY,
  order_id UUID NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
  method TEXT NOT NULL, -- 'card','cash'
  amount_cents INT NOT NULL,
  status TEXT NOT NULL, -- 'authorized','captured','voided','failed'
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_orders_cashier ON orders(cashier_id, created_at);
CREATE INDEX idx_products_sku ON products(sku);
```

Optional Future:

- `inventory_ledger` table for stock movements
- `returns` referencing original `order_id`

## 3. Domain Modules (Rust Crates / Folders)

Service: `pos-service` (new) OR extend `order-service` (Recommendation: new `pos-service` for cashier UX separation, later consolidation possible).

Modules:

- `catalog`: product lookup, tax_code retrieval
- `tax`: compute_tax(subtotal_cents, tax_code) -> tax_cents
- `discount`: apply_cart_discount(subtotal, percent_basis_points) -> (discount_cents, discounted_subtotal)
- `cart`: in-memory struct (frontend) – backend only validates at order creation
- `orders`: create_order, persist_items, finalize_totals
- `payments`: mock processor trait + implementations (CardMock, CashImmediate)
- `receipts`: format_plaintext(order, items, payment) + json view
- `events`: emit_pos_order(order_event) (feature gate `kafka` optional)

## 4. API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | /products?sku= | Lookup product by SKU/barcode |
| POST | /orders | Create order + (optional inline payment) |
| GET | /orders/{id} | Fetch order detail |
| POST | /orders/{id}/payments | Add payment (if not inline) |
| GET | /orders/{id}/receipt | Get receipt text/json |
| POST | /orders/{id}/void (manager) | Mark order void (future) |
| POST | /orders/{id}/refund (manager) | Refund (future) |

POST /orders Request (MVP):

```json
{
  "items": [{ "sku": "ABC123", "qty": 2 }],
  "discount_percent_bp": 500, // 5.00% optional
  "payment": { "method": "cash", "amount_cents": 3250 },
  "cashier_id": "...uuid..."
}
```

Response:

```json
{
  "order_id": "...",
  "status": "paid",
  "totals": { "subtotal_cents": 3000, "discount_cents": 150, "tax_cents": 285, "total_cents": 3135 },
  "payment": { "method": "cash", "amount_cents": 3250, "change_cents": 115 },
  "receipt_text": "..."
}
```

## 5. Tax & Discount Logic

Assumptions:

- Each product has a single tax_code mapping to rate (basis points: 825 = 8.25%).
- Tax computed after discount (cart-level percent) proportionally across lines.

Algorithm:

1. Sum line_subtotals = Σ(price_cents * qty)
2. discount_cents = floor(subtotal * discount_bp / 10_000)
3. discounted_subtotal = subtotal - discount_cents
4. For each line: proportional_share = (line_subtotal / subtotal) * discount_cents (round via banker’s or floor) → line_discount
5. line_tax_base = line_subtotal - line_discount
6. line_tax_cents = floor(line_tax_base * tax_rate_bp / 10_000)
7. total_tax_cents = Σ line_tax_cents
8. total_cents = discounted_subtotal + total_tax_cents

Edge Cases:

- Zero subtotal (empty items) → reject
- Discount > subtotal → clamp at subtotal
- Negative qty → reject
- Unknown SKU → reject with code `unknown_sku`

## 6. Payment Handling

Card (mock): Always “captured” unless amount mismatch.
Cash: amount_cents must be >= total_cents. change_cents = amount - total.
Failure Conditions:

- Provided amount < total (cash) → 400 `insufficient_cash`
- Card mismatch (amount != total) → 400 `amount_mismatch`

## 7. Event Emission

Topic: `pos.order`
Payload (MVP):

```json
{
  "order_id": "...",
  "status": "paid",
  "total_cents": 3135,
  "tax_cents": 285,
  "discount_cents": 150,
  "items": [{"sku":"ABC123","qty":2,"unit_price_cents":1500}],
  "payment_method": "cash",
  "occurred_at": "2025-10-02T12:34:56Z"
}
```

Feature Gate: build with `--features kafka` to enable, otherwise no-op.

## 8. Receipt Format (Plaintext MVP)

```text
NovaPOS Receipt
Order: 1234-...  Date: 2025-10-02 12:34
-------------------------------------
Qty  SKU      Price   Line
2    ABC123   15.00   30.00
-------------------------------------
Subtotal:          30.00
Discount (5%):     -1.50
Tax:                2.85
Total:             31.35
Paid (Cash):       32.50
Change:             1.15
-------------------------------------
Thank you!
```

## 9. Security / Roles (Minimal)

- JWT claim `role`: `cashier` or `manager`.
- Only `manager` can void / refund (endpoints may return 403 for now if invoked pre-implementation).
- No deep capability engine until after MVP adoption.

## 10. Offline Strategy (Design Stub)

- Introduce local queue abstraction: `OfflineOp { kind: CreateOrder, payload }`.
- If POST /orders fails with network, persist to IndexedDB (frontend) or local SQLite (native) for later sync.
- Sync endpoint: POST /offline/replay (future).

## 11. Testing Strategy

Unit:

- tax::compute_tax scenarios (rounding, proportional discount)
- discount::apply_cart_discount boundary (0%, 100%)
- payment::cash_change
Integration:

- create order success (cash + exact amount)
- create order failure (unknown SKU)
- cash insufficient
- card mismatch
- discount rounding distribution
Event:

- build with `kafka` feature and set capture env (if using existing harness style) → assert serialized payload shape.

## 12. Implementation Order (One Sprint)

Day 1: Migrations + domain structs (Product, Order, OrderItem, Payment)
Day 2: Tax + discount modules + tests
Day 3: Order creation endpoint (no payment) + validation
Day 4: Payment integration (inline + separate endpoint) + cash change calc
Day 5: Event emission + receipt formatter
Day 6: Negative tests + edge cases (rounding, mismatch) + doc polish
Day 7: Buffer / hardening / minimal PWA UI stub (optional)

## 13. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Rounding discrepancies | Centralize rounding helpers; test known cases |
| Discount distribution drift | Proportional allocation test with sum equality assertion |
| Event coupling too early | Single topic only; add version field later if schema evolves |
| Overbuilding offline | Defer actual sync; only interface stub now |
| Role expansion churn | Hard-code minimal roles; design extension point in auth extractor |

## 14. Success Criteria

- Create order with mock data returns correct totals & receipt
- Payment captured (cash & card) reflected in DB
- Event emitted (or safely skipped without feature)
- All unit + integration tests pass in CI (< 2s for domain tests)
- No external infra (Kafka, Redis) required for default build

## 15. Follow-Up Backlog Seeds

- POS Returns (reference original order items, restock integration)
- Loyalty accrual + redemption (points ledger)
- Promotion codes (stack rules) framework
- Multi-tax jurisdiction (destination-based breakdown)
- Real payment gateway adapter layer (Stripe/Adyen)
- Offline replay queue service endpoint

---
Prepared: 2025-10-02
