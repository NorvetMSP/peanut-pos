import type { DeviceInfo, DeviceStatus, Result } from "../types";
import type { PrinterCapabilities, PrinterDevice, PrintJob } from "../printer";

export interface MockPrinterOptions {
  failNext?: boolean;
}

export class MockPrinter implements PrinterDevice {
  public kind = "printer" as const;
  private failNext = false;

  constructor(opts?: MockPrinterOptions) {
    this.failNext = !!opts?.failNext;
  }

  async info(): Promise<DeviceInfo> {
    return { vendor: "MockCo", model: "Printer-42", transport: "mock" };
  }

  async status(): Promise<DeviceStatus> {
    return this.failNext ? { state: "error", code: "io_error", message: "Injected failure" } : { state: "ready" };
  }

  async capabilities(): Promise<PrinterCapabilities> {
    return { widthChars: [32, 42], supportsQr: true };
  }

  async print(job: PrintJob): Promise<Result<void, "io_error" | "invalid_payload" | "device_unavailable" | "timeout">> {
    if (!job.blocks?.length) {
      return { ok: false, error: { code: "invalid_payload", message: "No blocks" } };
    }
    if (this.failNext) {
      this.failNext = false;
      return { ok: false, error: { code: "io_error", message: "Injected" } };
    }
    return { ok: true, value: undefined };
  }
}
