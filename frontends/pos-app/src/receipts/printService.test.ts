import { describe, it, expect } from 'vitest';
import { printSaleReceiptWithRetry, getPrinter } from './printService';
import type { SaleReceipt } from './format';

describe('printService retry', () => {
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

    const promise = printSaleReceiptWithRetry(receipt, { maxAttempts: 3, intervalMs: 100 });

    // After short delay, set printer to ready
    setTimeout(() => mock.__setStatus?.({ state: 'ready' }), 120);

    const res = await promise;
    expect(res.ok).toBe(true);
  });
});
