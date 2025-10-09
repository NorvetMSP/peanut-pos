import type { DeviceInfo, DeviceStatus, Result } from "./types";

export interface ScanEvent {
  data: string; // barcode text
  symbology?: string; // e.g., EAN-13, Code128
  ts: number; // epoch ms
}

export interface ScannerDevice {
  kind: "scanner";
  info(): Promise<DeviceInfo>;
  status(): Promise<DeviceStatus>;
  startListening(onScan: (ev: ScanEvent) => void): Promise<Result<void, "device_unavailable" | "permission_denied">>;
  stopListening(): Promise<void>;
}
