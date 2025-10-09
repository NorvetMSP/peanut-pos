# Device SDK (Cashier MVP) — Brief

Scope: Printer, Scanner, Payment Terminal abstractions for POS edge. MVP delivers TypeScript interfaces, error model, mock drivers, and no UI coupling yet.

Principles
- Stable contracts: minimal, composable interfaces with typed results
- Clear error model: small set of error codes; DeviceStatus for health
- Testability: mock devices produce deterministic outputs; no DOM APIs baked-in
- Future: hot-plug/events and retry policies added in P13-02

Paths
- POS: `frontends/pos-app/src/devices/*`
- Index exports all types and device interfaces; `mocks/*` provide test doubles

Interfaces
- PrinterDevice: `print(PrintJob)` with blocks (text/qr), capabilities
- ScannerDevice: `startListening(onScan)` → `ScanEvent`, `stopListening`
- PaymentTerminalDevice: `present(request)` → `TerminalResponse`, `cancel`

Error & status
- DeviceStatus union: disconnected | connecting | ready | busy | error
- Result<T> tagged union with error codes: device_unavailable, timeout, permission_denied, invalid_payload, io_error, not_supported

Next (P13-02)
- Hot-plug detection; retry/backoff; telemetry hooks; device selection registry

Acceptance for today
- TS compiles; mocks added; no UI wiring
