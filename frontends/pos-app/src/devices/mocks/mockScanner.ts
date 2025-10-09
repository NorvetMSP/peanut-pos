import type { DeviceInfo, DeviceStatus, Result } from "../types";
import type { ScanEvent, ScannerDevice } from "../scanner";

export class MockScanner implements ScannerDevice {
  public kind = "scanner" as const;
  private listener: ((ev: ScanEvent) => void) | null = null;

  async info(): Promise<DeviceInfo> {
    return { vendor: "MockCo", model: "Scanner-1", transport: "mock" };
  }

  async status(): Promise<DeviceStatus> {
    return { state: this.listener ? "ready" : "disconnected" };
  }

  async startListening(onScan: (ev: ScanEvent) => void): Promise<Result<void, "device_unavailable" | "permission_denied">> {
    this.listener = onScan;
    return { ok: true, value: undefined };
  }

  async stopListening(): Promise<void> {
    this.listener = null;
  }

  // helper to simulate a scan
  emit(data: string, symbology = "EAN-13") {
    this.listener?.({ data, symbology, ts: Date.now() });
  }
}
