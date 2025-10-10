# POS Receipts (MVP)

Scope: Print-only receipts via Device SDK; e-receipts are deferred. We format a simple text receipt sized to the printer width and send to the selected device.

Highlights

- Device SDK abstraction: printer exposes capabilities (supported widths) and a `print(job)` method.
- Formatter: `buildSaleReceiptJob(data, width)` returns a PrintJob of text blocks.
- Service: `printSaleReceipt(data)` picks width from capabilities and prints.
- UX: On successful sale, POS tries to auto-print. A "Print Receipt" button supports manual reprints.
- Failure UX: If printing fails, a small toast appears with a Retry action. Clicking Retry resends the last receipt payload.
- Offline/Retry UX: If the printer is unavailable, the receipt is queued and retried automatically on reconnect (with backoff). A toast appears: "Printer offline — receipt queued for retry". Success/failure toasts follow when attempts conclude.
- Branding: `brandName` and `brandHeaderLines` render at the top of the receipt. Preferred source is tenant-config via Auth Service; env variables serve as fallback.
- Success indicator: On successful print, a brief green toast (“Printed”) confirms completion.

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
- Admin settings for per-tenant branding and footer lines (resolved via `resolveBranding(tenantId, token)`).
- Proactive printer status banner to surface disconnect/error states is present in MVP using event-driven updates.
- Printer discovery and selection UI; integration with native bridges for hardware printers.

Telemetry

- POS increments the following counters/gauges during retry flows:
  - `pos.print.retry.queued` (counter)
  - `pos.print.retry.success` (counter)
  - `pos.print.retry.failed` (counter)
  - `pos.print.queue_depth` (gauge)
  - `pos.print.retry.last_attempt` (gauge: ms timestamp)
- Console logging can be enabled with `VITE_ENABLE_CONSOLE_TELEMETRY=true`.
- Optional ingestion: set `VITE_TELEMETRY_INGEST_URL` and `VITE_TELEMETRY_MIN_INTERVAL_MS` to POST snapshots to a backend endpoint.
- A small Device Diagnostics panel in Cashier shows current printer status, queue depth, and last retry time.

Branding resolution

- The POS attempts to resolve branding from the Auth Service if available, merging results with env fallback:
  - `resolveBranding(tenantId, token)` returns `{ brandName?, brandHeaderLines? }`.
  - Env fallback variables are still supported and can be set locally.

Environment variables (POS)

- `VITE_BRAND_NAME` — Optional brand name to render as the receipt title.
- `VITE_BRAND_HEADER_LINES` — Optional pipe-separated extra header lines, e.g.
  - `VITE_BRAND_HEADER_LINES="123 Market St.|Anytown, CA|(555) 555-1212"`
- `VITE_ENABLE_CONSOLE_TELEMETRY` — Enable console debug logs for counters/gauges.
- `VITE_TELEMETRY_INGEST_URL` — When set, the POS will periodically POST telemetry snapshots to this URL.
- `VITE_TELEMETRY_MIN_INTERVAL_MS` — Minimum interval between telemetry flushes.
