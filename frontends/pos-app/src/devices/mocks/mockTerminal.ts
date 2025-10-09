import type { DeviceInfo, DeviceStatus, Result } from "../types";
import type { PaymentTerminalDevice, TerminalRequest, TerminalResponse } from "../terminal";

export interface MockTerminalOptions {
  approve?: boolean;
  providerRef?: string;
}

export class MockTerminal implements PaymentTerminalDevice {
  public kind = "terminal" as const;
  private approve: boolean;
  private providerRef?: string;

  constructor(opts?: MockTerminalOptions) {
    this.approve = opts?.approve ?? true;
    this.providerRef = opts?.providerRef ?? "mock_ref_123";
  }

  async info(): Promise<DeviceInfo> {
    return { vendor: "MockCo", model: "Terminal-1", transport: "mock" };
  }

  async status(): Promise<DeviceStatus> {
    return { state: "ready" };
  }

  async present(req: TerminalRequest): Promise<Result<TerminalResponse, "timeout" | "device_unavailable" | "io_error">> {
    if (this.approve) {
      return {
        ok: true,
        value: {
          requestId: req.requestId,
          approved: true,
          approvalCode: "APPROVED",
          maskedPan: "**** **** **** 4242",
          cardBrand: "VISA",
          providerRef: this.providerRef,
        },
      };
    }
    return {
      ok: true,
      value: {
        requestId: req.requestId,
        approved: false,
        errorReason: "DECLINED",
      },
    };
  }

  async cancel(): Promise<Result<void, "not_supported" | "io_error">> {
    return { ok: false, error: { code: "not_supported" } };
  }
}
