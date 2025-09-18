import { renderHook, act } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { CartProvider, useCart } from './CartContext';

describe('CartContext', () => {
  const wrapper: React.FC<{ children: React.ReactNode }> = ({ children }) => (
    <CartProvider>{children}</CartProvider>
  );

  beforeEach(() => {
    window.localStorage.clear();
  });

  it('adds items and updates quantities correctly', () => {
    const { result } = renderHook(() => useCart(), { wrapper });

    expect(result.current.cart).toEqual([]);
    expect(result.current.totalAmount).toBe(0);

    act(() => {
      result.current.addItem({ id: 'prod1', name: 'Test Product', price: 10 });
    });

    expect(result.current.cart).toHaveLength(1);
    expect(result.current.cart[0].quantity).toBe(1);
    expect(result.current.totalAmount).toBe(10);

    act(() => {
      result.current.addItem({ id: 'prod1', name: 'Test Product', price: 10 });
    });

    expect(result.current.cart).toHaveLength(1);
    expect(result.current.cart[0].quantity).toBe(2);
    expect(result.current.totalAmount).toBe(20);

    act(() => {
      result.current.incrementItemQuantity('prod1');
    });

    expect(result.current.cart[0].quantity).toBe(3);

    act(() => {
      result.current.decrementItemQuantity('prod1');
    });

    expect(result.current.cart[0].quantity).toBe(2);
    expect(result.current.totalAmount).toBe(20);

    act(() => {
      result.current.removeItem('prod1');
    });

    expect(result.current.cart).toEqual([]);
    expect(result.current.totalAmount).toBe(0);
  });
});
