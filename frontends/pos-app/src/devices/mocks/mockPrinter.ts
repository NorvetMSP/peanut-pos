import type { DeviceInfo, DeviceStatus, Result } from "../types";
import type { PrinterCapabilities, PrinterDevice, PrintJob } from "../printer";

export interface MockPrinterOptions {
  failNext?: boolean;
}

export class MockPrinter implements PrinterDevice {
  public kind = "printer" as const;
  private failNext = false;
  private listeners: Array<(s: DeviceStatus) => void> = [];
  private currentStatus: DeviceStatus = { state: 'ready' };

  constructor(opts?: MockPrinterOptions) {
    this.failNext = !!opts?.failNext;
  }

  async info(): Promise<DeviceInfo> {
    return { vendor: "MockCo", model: "Printer-42", transport: "mock" };
  }

  async status(): Promise<DeviceStatus> { return this.currentStatus; }

  // Test helper to simulate status changes
  public __setStatus(status: DeviceStatus) {
    this.currentStatus = status;
    for (const l of this.listeners) l(status);
  }

  async capabilities(): Promise<PrinterCapabilities> {
    return { widthChars: [32, 42], supportsQr: true };
  }

  async print(job: PrintJob): Promise<Result<void, "io_error" | "invalid_payload" | "device_unavailable" | "timeout">> {
    if (!job.blocks?.length) {
      return { ok: false, error: { code: "invalid_payload", message: "No blocks" } };
    }
    if (this.currentStatus.state !== 'ready') {
      return { ok: false, error: { code: 'device_unavailable', message: this.currentStatus.state } };
    }
    if (this.failNext) {
      this.failNext = false;
      return { ok: false, error: { code: "io_error", message: "Injected" } };
    }
    return { ok: true, value: undefined };
  }

  on?(event: 'status', handler: (status: DeviceStatus) => void): () => void {
    if (event !== 'status') return () => {};
    this.listeners.push(handler);
    // Emit current immediately so subscribers have a baseline
    handler(this.currentStatus);
    return () => {
      this.listeners = this.listeners.filter(h => h !== handler);
    };
  }
}
