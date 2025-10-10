import { MockPrinter } from "../devices/mocks/mockPrinter";
import type { PrinterDevice } from "../devices/printer";
import { buildSaleReceiptJob, type SaleReceipt } from "./format";

let cached: PrinterDevice | null = null;

export async function getPrinter(): Promise<PrinterDevice> {
  if (cached) return cached;
  // MVP: use MockPrinter; later detect real hardware
  cached = new MockPrinter();
  return cached;
}

export async function printSaleReceipt(data: SaleReceipt): Promise<{ ok: boolean; message?: string }> {
  try {
    const printer = await getPrinter();
    const caps = await printer.capabilities();
    const width = caps.widthChars?.[0] ?? 42;
    const job = buildSaleReceiptJob(data, width);
    const res = await printer.print(job);
    if (!res.ok) return { ok: false, message: res.error.message ?? res.error.code };
    return { ok: true };
  } catch (err) {
    return { ok: false, message: err instanceof Error ? err.message : String(err) };
  }
}
