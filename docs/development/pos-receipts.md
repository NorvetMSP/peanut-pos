# POS Receipts (MVP)

Scope: Print-only receipts via Device SDK; e-receipts are deferred. We format a simple text receipt sized to the printer width and send to the selected device.

Highlights

- Device SDK abstraction: printer exposes capabilities (supported widths) and a `print(job)` method.
- Formatter: `buildSaleReceiptJob(data, width)` returns a PrintJob of text blocks.
- Service: `printSaleReceipt(data)` picks width from capabilities and prints.
- UX: On successful sale, POS tries to auto-print. A "Print Receipt" button supports manual reprints.
- Failure UX: If printing fails, a small toast appears with a Retry action. Clicking Retry resends the last receipt payload.
- Branding: Optional `brandName` and `brandHeaderLines` render at the top of the receipt.

API shapes

- `SaleReceipt`:
  - brandName?: string
  - brandHeaderLines?: string[]
  - storeLabel: string
  - cashierLabel: string
  - items: CartItem[]
  - subtotal: number
  - tax?: number
  - total: number
  - paidMethod: string
  - createdAt: Date
  - footerNote?: string

Future work

- Add QR for “scan for e-receipt” once email/SMS opt-in exists.
- Admin settings for per-tenant branding and footer lines.
- Printer discovery and selection UI; integration with native bridges for hardware printers.
