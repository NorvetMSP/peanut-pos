import type { DeviceInfo, DeviceStatus, Result } from "./types";

export type TenderType = "card" | "cash" | "qr" | "other";

export interface TerminalAmount {
  currency: string; // ISO 4217
  amountMinor: number; // e.g., cents
}

export interface TerminalRequest {
  requestId: string;
  amount: TerminalAmount;
  tipAmountMinor?: number;
  metadata?: Record<string, unknown>;
}

export interface TerminalResponse {
  requestId: string;
  approved: boolean;
  approvalCode?: string;
  maskedPan?: string;
  cardBrand?: string;
  providerRef?: string;
  errorReason?: string;
}

export interface PaymentTerminalDevice {
  kind: "terminal";
  info(): Promise<DeviceInfo>;
  status(): Promise<DeviceStatus>;
  present(amount: TerminalRequest): Promise<Result<TerminalResponse, "timeout" | "device_unavailable" | "io_error">>;
  cancel(): Promise<Result<void, "not_supported" | "io_error">>;
}
