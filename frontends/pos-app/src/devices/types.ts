// Common device SDK types for POS edge
// Scope: MVP skeleton interfaces and error model, no runtime wiring

export type DeviceStatus =
  | { state: "disconnected"; reason?: string }
  | { state: "connecting" }
  | { state: "ready" }
  | { state: "busy"; task?: string }
  | { state: "error"; code: DeviceErrorCode; message?: string };

export type DeviceErrorCode =
  | "device_unavailable"
  | "timeout"
  | "permission_denied"
  | "invalid_payload"
  | "io_error"
  | "not_supported";

export type Result<T, E extends string = string> =
  | { ok: true; value: T }
  | { ok: false; error: { code: E; message?: string } };

export type RetryPolicy =
  | { kind: "none" }
  | { kind: "linear"; intervalMs: number; maxAttempts?: number }
  | { kind: "exponential"; baseMs: number; factor?: number; maxAttempts?: number };

export interface DeviceInfo {
  vendor?: string;
  model?: string;
  serial?: string;
  transport?: "usb" | "ble" | "network" | "mock" | "unknown";
}
