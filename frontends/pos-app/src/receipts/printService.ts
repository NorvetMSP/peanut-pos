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

// Retry queue (simple, in-memory per-session)
type QueueItem = { job: ReturnType<typeof buildSaleReceiptJob>; resolve: (r: { ok: boolean; message?: string }) => void; attempts: number };
const queue: QueueItem[] = [];
let unsubscribe: (() => void) | null = null;

export async function printSaleReceiptWithRetry(data: SaleReceipt, opts?: { maxAttempts?: number; intervalMs?: number }): Promise<{ ok: boolean; message?: string }> {
  const maxAttempts = Math.max(1, opts?.maxAttempts ?? 3);
  const intervalMs = Math.max(500, opts?.intervalMs ?? 1500);

  const printer = await getPrinter();
  const width = (await printer.capabilities()).widthChars?.[0] ?? 42;
  const job = buildSaleReceiptJob(data, width);

  // Try immediate print first
  const first = await printer.print(job);
  if (first.ok) return { ok: true };
  if (first.error.code !== 'device_unavailable') {
    return { ok: false, message: first.error.message ?? first.error.code };
  }

  // Device unavailable: queue and subscribe for status changes
  return new Promise<{ ok: boolean; message?: string }>(resolve => {
    queue.push({ job, resolve, attempts: 1 });

    const tryDequeue = async () => {
      if (queue.length === 0) return;
      const head = queue[0];
      const res = await printer.print(head.job);
      if (res.ok) {
        head.resolve({ ok: true });
        queue.shift();
        return;
      }
      head.attempts += 1;
      if (head.attempts > maxAttempts) {
        head.resolve({ ok: false, message: res.error.message ?? res.error.code });
        queue.shift();
        return;
      }
      // Backoff and re-attempt later
      setTimeout(tryDequeue, intervalMs);
    };

    if (!unsubscribe && typeof printer.on === 'function') {
      unsubscribe = printer.on('status', s => {
        if (s.state === 'ready') {
          void tryDequeue();
        }
      });
    }
  });
}
