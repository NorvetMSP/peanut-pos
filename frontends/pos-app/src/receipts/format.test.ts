import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { buildSaleReceiptJob, type SaleReceipt } from './format';

const realToLocaleString = Date.prototype.toLocaleString;

describe('buildSaleReceiptJob', () => {
  beforeAll(() => {
    // Stabilize date formatting for snapshot consistency
    // eslint-disable-next-line no-extend-native
    Date.prototype.toLocaleString = function (this: Date) {
      const y = this.getUTCFullYear();
      const m = String(this.getUTCMonth() + 1).padStart(2, '0');
      const d = String(this.getUTCDate()).padStart(2, '0');
      const hh = String(this.getUTCHours()).padStart(2, '0');
      const mm = String(this.getUTCMinutes()).padStart(2, '0');
      const ss = String(this.getUTCSeconds()).padStart(2, '0');
      return `${y}-${m}-${d} ${hh}:${mm}:${ss} UTC`;
    } as any;
  });

  afterAll(() => {
    Date.prototype.toLocaleString = realToLocaleString;
  });

  const sample: SaleReceipt = {
    orderId: '11111111-1111-1111-1111-111111111111',
    storeLabel: 'Flagship Store',
    cashierLabel: 'Casey Cashier',
    items: [
      { id: 'p1', name: 'Premium Coffee Beans 1kg', price: 14.99, quantity: 1, sku: 'COF-1KG' },
      { id: 'p2', name: 'Reusable Cup', price: 9.5, quantity: 2, sku: 'CUP-R' },
    ],
    subtotal: 14.99 + 9.5 * 2,
    total: 14.99 + 9.5 * 2 + 2.34,
    tax: 2.34,
    paidMethod: 'card',
    createdAt: new Date(Date.UTC(2025, 0, 2, 3, 4, 5)),
    footerNote: 'Thanks for shopping!',
  };

  it('builds a job at width 42', () => {
    const job = buildSaleReceiptJob(sample, 42);
    expect(job.widthChars).toBe(42);
    expect(job.blocks.length).toBeGreaterThan(0);
    expect(job).toMatchSnapshot();
  });

  it('builds a job at width 32', () => {
    const job = buildSaleReceiptJob(sample, 32);
    expect(job.widthChars).toBe(32);
    expect(job.blocks.length).toBeGreaterThan(0);
    expect(job).toMatchSnapshot();
  });
});
