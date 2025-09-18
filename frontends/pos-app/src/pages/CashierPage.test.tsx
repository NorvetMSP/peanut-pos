import React from 'react';
import { describe, beforeEach, afterEach, expect, it, vi } from 'vitest';
import { render, fireEvent, waitFor, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { AuthProvider } from '../AuthContext';
import { OrderProvider } from '../OrderContext';
import { CartProvider } from '../CartContext';
import CashierPage from './CashierPage';

describe('CashierPage integration flows', () => {
  beforeEach(() => {
    window.localStorage.clear();
    const session = { token: 'token-123', user: { tenant_id: 'tenant-test' }, timestamp: Date.now() };
    window.localStorage.setItem('session', JSON.stringify(session));
    if (typeof window.HTMLIFrameElement === 'undefined') {
      Object.defineProperty(window, 'HTMLIFrameElement', { value: class {}, configurable: true });
    }
    if (typeof window.HTMLFrameElement === 'undefined') {
      Object.defineProperty(window, 'HTMLFrameElement', { value: class {}, configurable: true });
    }
    Object.defineProperty(window.navigator, 'onLine', { value: true, configurable: true });
    vi.spyOn(window, 'open').mockImplementation(() => null);

    const fetchMock = vi.fn((input: RequestInfo | URL) => {
      const url = typeof input === 'string' || input instanceof URL ? input.toString() : input.url;
      if (url.includes('/products')) {
        const body = [
          { id: 'p1', name: 'Product One', price: 10, description: '', active: true },
          { id: 'p2', name: 'Product Two', price: 20, description: '', active: true },
          { id: 'pX', name: 'Old Product', price: 15, description: '', active: false },
        ];
        return Promise.resolve(
          new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } }),
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

  const renderWithProviders = () =>
    render(
      <MemoryRouter initialEntries={['/pos']}>
        <AuthProvider>
          <OrderProvider>
            <CartProvider>
              <Routes>
                <Route path="/pos" element={<CashierPage />} />
              </Routes>
            </CartProvider>
          </OrderProvider>
        </AuthProvider>
      </MemoryRouter>,
    );

  it('loads products, filters, queues offline submission, and submits online', async () => {
    Object.defineProperty(window.navigator, 'onLine', { value: false, configurable: true });
    window.localStorage.setItem('pos-cart', JSON.stringify([{ id: 'pX', name: 'Old Product', price: 15, quantity: 1 }]));

    const view = renderWithProviders();

    await screen.findByText('Product One');

    const searchInput = screen.getByPlaceholderText('Search products...');
    fireEvent.change(searchInput, { target: { value: 'Product One' } });
    expect(screen.queryByText('Product Two')).toBeNull();
    fireEvent.change(searchInput, { target: { value: '' } });

    const submitButton = await screen.findByText('Submit Sale');
    fireEvent.click(submitButton);

    await screen.findByText('Item Unavailable');
    fireEvent.change(screen.getByLabelText('Replace with'), { target: { value: 'p1' } });
    fireEvent.click(screen.getByText('Replace'));

    await waitFor(() => {
      const ordersButton = screen.getByText(/Orders/);
      expect(ordersButton.textContent).toContain('(1)');
    });

    Object.defineProperty(window.navigator, 'onLine', { value: true, configurable: true });
    window.dispatchEvent(new Event('online'));

    await waitFor(() => {
      const ordersButton = screen.getByText(/Orders/);
      expect(ordersButton.textContent).not.toContain('(');
    });

    await new Promise(resolve => setTimeout(resolve, 1200));
    fireEvent.click(screen.getAllByText('Add')[0]);
    fireEvent.click(screen.getByText('Submit Sale'));

    await waitFor(() => {
      expect(screen.getByText('Your cart is empty.')).toBeTruthy();
    });

    fireEvent.click(screen.getByText(/Orders/));
    await screen.findByText(/Ref:/);

    view.unmount();
  });
});
