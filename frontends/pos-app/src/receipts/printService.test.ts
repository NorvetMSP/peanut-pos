import { describe, it, expect, beforeEach } from 'vitest';
import { printSaleReceiptWithRetry, getPrinter } from './printService';
import type { SaleReceipt } from './format';
import { getSnapshot, resetTelemetry } from '../services/telemetry';

describe('printService retry', () => {
  beforeEach(() => {
    resetTelemetry();
  });
  it('queues when unavailable and prints on ready status', async () => {
    const printer = await getPrinter();
    // Force mock to unavailable
    const mock = printer as any;
    mock.__setStatus?.({ state: 'disconnected' });

    const receipt: SaleReceipt = {
      storeLabel: 'Store',
      cashierLabel: 'Cashier',
      items: [{ id: 'p1', name: 'Item', price: 1.0, quantity: 1 } as any],
      subtotal: 1.0,
      total: 1.0,
      paidMethod: 'cash',
      createdAt: new Date(),
    };

  let queuedCalled = false;
  const promise = printSaleReceiptWithRetry(receipt, { maxAttempts: 3, intervalMs: 100, onQueued: () => { queuedCalled = true; } });

    // After short delay, set printer to ready
    setTimeout(() => mock.__setStatus?.({ state: 'ready' }), 120);

    const res = await promise;
    expect(res.ok).toBe(true);
    expect(queuedCalled).toBe(true);
    const snap = getSnapshot();
    // At the end, queue depth should be 0
    const depth = Array.from(snap.gauges.entries()).find(([k]) => k.startsWith('pos.print.queue_depth'))?.[1] ?? 0;
    expect(depth).toBe(0);
  });
});
