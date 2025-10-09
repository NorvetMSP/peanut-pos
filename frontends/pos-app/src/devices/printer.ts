import type { DeviceInfo, DeviceStatus, Result } from "./types";

export interface PrintTextBlock {
  type: "text";
  content: string;
  align?: "left" | "center" | "right";
  bold?: boolean;
  size?: "s" | "m" | "l";
}

export interface PrintQrBlock {
  type: "qr";
  data: string;
  size?: number; // pixels or device-specific units
}

export type PrintBlock = PrintTextBlock | PrintQrBlock;

export interface PrintJob {
  widthChars?: number; // e.g., 32/42/48 columns
  blocks: PrintBlock[];
  cut?: boolean;
}

export interface PrinterCapabilities {
  widthChars?: number[];
  supportsQr?: boolean;
}

export interface PrinterDevice {
  kind: "printer";
  info(): Promise<DeviceInfo>;
  status(): Promise<DeviceStatus>;
  capabilities(): Promise<PrinterCapabilities>;
  print(job: PrintJob): Promise<Result<void, "io_error" | "invalid_payload" | "device_unavailable" | "timeout">>;
}
