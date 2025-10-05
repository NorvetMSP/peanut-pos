# POS MVP Build Plan (Updated)

Focused objective: Deliver a cashier-facing flow (Scan / Add Item → View Cart + Totals → Take Payment → Persist Order → Emit Event → Receipt) in the smallest coherent vertical slice.

Note: Implementation is consolidated into the existing order-service (Axum + SQLx). JWT auth is required; callers must send X-Tenant-ID, X-Roles, and Authorization: Bearer `<jwt>`.

## MVP status at a glance (2025-10-05)

- [x] Product lookup by SKU/barcode
- [x] Cart operations (add/remove/qty)
- [x] Tax calculation (flat rate per tax_code)
- [x] Discount (cart-level %)
- [x] Payment capture (mock card + cash) with change due
- [x] Order persistence (orders, items, payments)
- [x] Receipt generation (plaintext + JSON data)
- [x] Event emission `pos.order` (feature-gated via `kafka`)
- [x] RBAC enforcement (Cashier/Manager/Admin); E2E verified
- [x] Z-report settlement by tender (`GET /reports/settlement`); admin page added; integration + E2E coverage

Delivered early from “Should Have”:

- [x] Partial refunds (per-item quantities) and returns listing
- [x] Inventory decrement/enforcement via inventory-service
- [x] Loyalty earn on order.completed (simple accrual)

Still pending from “Should Have”:

- [ ] CI running frontend E2Es in PRs (Playwright)

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

- Returns / exchanges referencing original order (returns + partial refunds delivered; exchanges flow pending)
- Inventory decrement + low-stock event stub (enforcement delivered; low-stock event pending)
- Loyalty points accrue (delivered: simple earn on order.completed)
- End-of-day settlement (Z-report by tender type) — delivered early
- Partial refunds — delivered early

Status update (2025-10-05):

- Inventory enforcement is ON (compose has no bypass); order-service forwards Admin/Manager/Cashier roles to inventory-service for reservations.
- Returns/refunds: POST /orders/refund implemented; GET /returns lists refunds. Partial refunds supported via per-item quantities.
- Z-report: GET /reports/settlement?date=YYYY-MM-DD aggregates captured payments by method for the date; indexed for performance.
- Exchanges: POST /orders/{id}/exchange orchestrates returns + replacement order; links via `orders.exchange_of_order_id`; integration tests added.
- Low stock: inventory-service emits `inventory.low_stock` only when crossing from above threshold to at/below threshold (prevents spam while below).
- Loyalty: upsert on order.completed is present in loyalty-service; simple earn path implemented.

Frontend status (2025-10-05):

- Admin Portal: SettlementReportPage available under `/reports/settlement` with manager/admin guard; totals formatted as currency; RBAC E2E tests passing; date filter E2E passing.
- POS App: Cashier flow E2E (cash) passing; Card redirect and Card failure E2Es passing (error banner + retry UX); ExchangePage supports selecting items from original order and submitting exchange; E2Es for basic and selection flows passing.

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

## 3. Domain Modules (Current services)

We extended `order-service` to serve the POS MVP:

- Catalog lookups via `products` table
- Tax precedence: POS override > Location override > Tenant default; product `tax_code` controls taxability
- Discount: cart-level percent supported; proportional allocation in compute
- Orders: create from SKUs (`/orders/sku`) and raw product IDs (`/orders`), idempotency supported
- Payments: mock card and cash; change for cash; `payments` table records captured payments
- Receipts: plaintext (`/orders/{id}/receipt`) based on computed cents
- Events: `pos.order` and order lifecycle events (Kafka optional via feature flags)

## 4. API Endpoints (order-service)

| Method | Path | Description |
|--------|------|-------------|
| POST | /orders/compute | Compute totals for items, discount, tax precedence |
| POST | /orders | Create order from product payload |
| POST | /orders/sku | Create order from SKUs |
| GET | /orders | List orders (filters, date range) |
| GET | /orders/{id} | Fetch order detail |
| GET | /orders/{id}/receipt | Get receipt text |
| POST | /orders/{id}/void | Void an order (manager) |
| POST | /orders/refund | Refund items from an order (manager) |
| GET | /returns | List returns/refunds |
| GET | /reports/settlement?date=YYYY-MM-DD | Z-report totals by payment method |

Headers required:

- `X-Tenant-ID: <uuid>`
- `X-Roles: Admin|Manager|Cashier` (comma/space separated)
- `Authorization: Bearer <jwt>` (tid claim required)

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

- JWT must include `tid` (tenant id) and UUID `sub`.
- Header `X-Tenant-ID` must match `tid`.
- Roles header: `X-Roles` must include appropriate roles. Admin/Manager/Support can access admin/reporting endpoints; Cashier can create orders.
- Only Manager/Admin can void/refund.
- No deep capability engine until after MVP.

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
UI/E2E:

- Admin-portal RBAC for settlement report (cashier denied, manager allowed via UI)
- POS cashier smoke (cash) and card redirect (window.open) with network mocks
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
- Settlement report returns expected aggregates by method for a given date
- All unit + integration tests pass in CI
- No external infra (Kafka, Redis) required for default build

## 15. Follow-Up Backlog Seeds

- POS Returns: extend refund detail endpoint and exchange flow; restock via inventory-service on returns
- Loyalty accrual + redemption (points ledger)
- Promotion codes (stack rules) framework
- Multi-tax jurisdiction (destination-based breakdown)
- Real payment gateway adapter layer (Stripe/Adyen)
- Offline replay queue service endpoint

Related design doc:

- Exchanges API RFC: `docs/rfcs/exchanges_api.md`

### Windows/Dev Notes

- Local testing: set `TEST_DATABASE_URL` for integration tests. Use PowerShell scripts under `scripts/` to seed and demo flows.

```powershell
# From repo root
$env:TEST_DATABASE_URL = "postgres://postgres:postgres@localhost:5432/novapos_test"
pushd services; cargo test -p order-service --no-default-features --features integration-tests --tests -- --test-threads=1; popd
```

- JWT: `scripts\mint-dev-jwt.js` can mint dev tokens; for tests where we inject a dev key, set `JWT_DEV_PUBLIC_KEY_PEM`.

```powershell
# Example: mint a token (requires Node.js)
node .\scripts\mint-dev-jwt.js -t <tenant-uuid> -r Admin,Cashier -a novapos-admin -i https://auth.novapos.local
```

- Inventory:  compose no longer sets `ORDER_BYPASS_INVENTORY`; service forwards `X-Roles: Admin,Manager,Cashier` to inventory-service.
- SQLx offline: use `regenerate-sqlx-data.ps1` to refresh query metadata across crates.

Optional: Frontend tests (run from each app folder):

```powershell
# Admin Portal unit tests (Vitest)
pushd .\frontends\admin-portal; npm test --silent; popd

# Admin Portal E2E (Playwright)
pushd .\frontends\admin-portal; npx playwright install; npx playwright test --workers=1; popd

# POS App E2E (Playwright)
pushd .\frontends\pos-app; npx playwright install; npx playwright test --workers=1; popd
```

Prepared: 2025-10-02 | Updated: 2025-10-05

## Summary of changes (this update)

- Consolidated plan to reflect actual `order-service` implementation (Axum + SQLx) with JWT and tenant headers.
- Added status update: inventory enforcement on; refunds/returns implemented; settlement (Z-report) endpoint live.
- Replaced the old pos-service sketch with accurate endpoints and flows (compute, create by SKUs, refund, returns, settlement report).
- Clarified security and roles (tid claim, X-Tenant-ID, X-Roles) and who can access admin/reporting endpoints.
- Polished Windows developer steps with PowerShell commands for tests and JWT minting.

Next documentation improvements (optional):

- Add curl examples for compute, create (SKU), refund, and Z-report.
- Link to runbook “Demo 3: inventory-on” section and scripts to create seed data.
- Include a JSON example for a return record (`/returns`) and a sample receipt output for parity with tests.

---
Prepared: 2025-10-02

## Quick curl examples (PowerShell)

Prereqs: replace placeholders for $tenant, $token, and service URL. On Windows PowerShell, use backtick ` for line continuation if desired; below are single-line commands for copy/paste.

Compute totals:

```powershell
$tenant = "<tenant-uuid>"; $token = "<jwt>"; $body = '{"items":[{"sku":"SKU-ABC","quantity":2}],"discount_percent_bp":500,"tax_rate_bps":800}';
curl -Method POST -Uri http://localhost:8080/orders/compute -Headers @{ 'Content-Type'='application/json'; 'X-Tenant-ID'=$tenant; 'X-Roles'='admin'; 'Authorization'="Bearer $token" } -Body $body
```

Create order by SKUs:

```powershell
$tenant = "<tenant-uuid>"; $token = "<jwt>"; $body = '{"items":[{"sku":"SKU-ABC","qty":2}],"discount_percent_bp":500,"payment":{"method":"cash","amount_cents":3250},"cashier_id":"<cashier-uuid>"}';
curl -Method POST -Uri http://localhost:8080/orders/sku -Headers @{ 'Content-Type'='application/json'; 'X-Tenant-ID'=$tenant; 'X-Roles'='cashier'; 'Authorization'="Bearer $token" } -Body $body
```

Refund items from an order:

```powershell
$tenant = "<tenant-uuid>"; $token = "<jwt>"; $orderId = "<original-order-uuid>"; $body = '{"order_id":"'+$orderId+'","items":[{"product_id":"<product-uuid>","quantity":1}],"reason":"customer_return"}';
curl -Method POST -Uri http://localhost:8080/orders/refund -Headers @{ 'Content-Type'='application/json'; 'X-Tenant-ID'=$tenant; 'X-Roles'='manager'; 'Authorization'="Bearer $token" } -Body $body
```

Settlement (Z-report) for a date:

```powershell
$tenant = "<tenant-uuid>"; $token = "<jwt>"; $date = (Get-Date).ToString('yyyy-MM-dd');
curl -Method GET -Uri ("http://localhost:8080/reports/settlement?date="+$date) -Headers @{ 'X-Tenant-ID'=$tenant; 'X-Roles'='admin'; 'Authorization'="Bearer $token" }
```

Exchange (return + replacement order):

```powershell
$tenant = "<tenant-uuid>"; $token = "<jwt>"; $orig = "<original-order-uuid>";
$body = '{"return_items":[{"product_id":"<product-uuid>","qty":1}],"new_items":[{"sku":"SKU-NEW","qty":1}],"payment":{"method":"cash","amount_cents":0}}';
curl -Method POST -Uri ("http://localhost:8080/orders/"+$orig+"/exchange") -Headers @{ 'Content-Type'='application/json'; 'X-Tenant-ID'=$tenant; 'X-Roles'='manager'; 'Authorization'="Bearer $token" } -Body $body
```

Low stock behavior:

- The inventory-service emits `inventory.low_stock` only when stock crosses from above threshold to at/below threshold; it won’t spam while remaining below.

Create order from SKUs (cash):

```powershell
$tenant = "<tenant-uuid>"; $token = "<jwt>"; $body = '{"items":[{"sku":"SKU-ABC","quantity":1}],"payment_method":"cash","payment":{"method":"cash","amount_cents":1500}}';
curl -Method POST -Uri http://localhost:8080/orders/sku -Headers @{ 'Content-Type'='application/json'; 'X-Tenant-ID'=$tenant; 'X-Roles'='cashier'; 'Authorization'="Bearer $token" } -Body $body
```

Refund items from an order (manager):

```powershell
$tenant = "<tenant-uuid>"; $token = "<jwt>"; $orderId = "<order-uuid>"; $body = '{"order_id":"'+$orderId+'","items":[{"product_id":"<product-uuid>","quantity":1}],"reason":"customer_return"}';
curl -Method POST -Uri http://localhost:8080/orders/refund -Headers @{ 'Content-Type'='application/json'; 'X-Tenant-ID'=$tenant; 'X-Roles'='manager'; 'Authorization'="Bearer $token" } -Body $body
```

Z-report for a date:

```powershell
$tenant = "<tenant-uuid>"; $token = "<jwt>"; $date = (Get-Date).ToString('yyyy-MM-dd');
curl -Method GET -Uri "http://localhost:8080/reports/settlement?date=$date" -Headers @{ 'X-Tenant-ID'=$tenant; 'X-Roles'='manager'; 'Authorization'="Bearer $token" }
```
