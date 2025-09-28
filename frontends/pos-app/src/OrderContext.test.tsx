import { renderHook, act } from '@testing-library/react';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { AuthProvider } from './AuthContext';
import { OrderProvider, useOrders } from './OrderContext';
import type { DraftOrderPayload, SubmitOrderResult } from './OrderContext';

describe('OrderContext offline queue', () => {
  const wrapper: React.FC<{ children: React.ReactNode }> = ({ children }) => (
    <AuthProvider>
      <OrderProvider>{children}</OrderProvider>
    </AuthProvider>
  );

  beforeEach(() => {
    window.localStorage.clear();
    const session = { token: 'test-token', user: { tenant_id: 'tenant-123' }, timestamp: Date.now() };
    window.localStorage.setItem('session', JSON.stringify(session));
    Object.defineProperty(window.navigator, 'onLine', { value: false, configurable: true });

    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const url = typeof input === 'string' || input instanceof URL ? input.toString() : input.url;
      if (url.includes('/session')) {
        return Promise.resolve(
          new Response(
            JSON.stringify({ token: 'test-token', user: { tenant_id: 'tenant-123' } }),
            { status: 200, headers: { 'Content-Type': 'application/json' } },
          ),
        );
      }
      if (url.includes('/orders')) {
        return Promise.resolve(
          new Response(
            JSON.stringify({ id: 'order-123', status: 'Submitted' }),
            { status: 200, headers: { 'Content-Type': 'application/json' } },
          ),
        );
      }
      if (url.includes('/payments')) {
        return Promise.resolve(
          new Response(
            JSON.stringify({ status: 'pending', payment_url: 'http://pay.test/abc' }),
            { status: 200, headers: { 'Content-Type': 'application/json' } },
          ),
        );
      }
      return Promise.resolve(new Response(null, { status: 404 }));
    });
    vi.stubGlobal('fetch', fetchMock);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    window.localStorage.clear();
    Object.defineProperty(window.navigator, 'onLine', { value: true, configurable: true });
  });

  it('queues orders when offline and flushes on reconnect', async () => {
    const { result } = renderHook(() => useOrders(), { wrapper });

    await act(async () => {
      await Promise.resolve();
    });

    const draftOrder: DraftOrderPayload = {
      items: [{ product_id: 'p1', quantity: 1, unit_price: 5, line_total: 5 }],
      payment_method: 'cash',
      total: 5,
    };

    let submitResult: SubmitOrderResult | null = null;
    await act(async () => {
      submitResult = await result.current.submitOrder(draftOrder);
    });

    expect(submitResult).not.toBeNull();
    if (!submitResult) return;
    expect((submitResult as { status: string }).status).toBe('queued');

    Object.defineProperty(window.navigator, 'onLine', { value: true, configurable: true });

    await act(async () => {
      await result.current.retryQueue();
    });

    expect(result.current.queuedOrders).toHaveLength(0);
    const recentOrder = result.current.recentOrders.find(order => order.reference === 'order-123');
    expect(recentOrder).toBeDefined();
    expect(recentOrder?.offline).toBe(false);
  });
});



