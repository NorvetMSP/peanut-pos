import { describe, it, expect } from 'vitest';
import { MockPrinter } from './mockPrinter';

describe('MockPrinter events', () => {
  it('invokes status listeners on subscribe and on change', async () => {
    const mp = new MockPrinter();
    const seen: string[] = [];
    const off = mp.on?.('status', s => { seen.push(s.state); });
    mp.__setStatus({ state: 'disconnected' });
    off?.();
    expect(seen[0]).toBe('ready'); // immediate emit
    expect(seen.includes('disconnected')).toBe(true);
  });
});
