import { describe, it, expect } from 'vitest';
import { parseReport, formatCurrency } from './settlementHelpers';

describe('settlementHelpers', () => {
  it('parseReport tolerates strings and numbers', () => {
    const input = {
      date: '2025-10-05',
      totals: [
        { method: 'cash', count: 2, amount: 12.34 },
        { method: 'card', count: '3', amount: '45.67' },
        { method: '', count: 'x', amount: 'y' },
      ],
    };
    const result = parseReport(input);
    expect(result.date).toBe('2025-10-05');
    expect(result.totals.length).toBe(3);
    expect(result.totals[0]).toEqual({ method: 'cash', count: 2, amount: 12.34 });
    expect(result.totals[1]).toEqual({ method: 'card', count: 3, amount: 45.67 });
    expect(result.totals[2]).toEqual({ method: 'unknown', count: 0, amount: 0 });
  });

  it('formatCurrency prints USD by default', () => {
    expect(formatCurrency(0)).toBe('$0.00');
    expect(formatCurrency(12.3)).toBe('$12.30');
  });
});
