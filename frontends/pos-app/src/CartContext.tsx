// src/CartContext.tsx
import React, { createContext, useContext, useEffect, useState } from 'react';

interface Product {
  id: string;
  name: string;
  price: number;
  sku?: string | null;
}

interface CartItem extends Product {
  quantity: number;
}

interface CartContextValue {
  cart: CartItem[];
  addItem: (product: Product) => void;
  removeItem: (productId: string) => void;
  clearCart: () => void;
  incrementItemQuantity: (productId: string) => void;
  decrementItemQuantity: (productId: string) => void;
  updateItemPrice: (productId: string, price: number) => void;
  totalAmount: number;
}

const CartContext = createContext<CartContextValue | undefined>(undefined);

const CART_STORAGE_KEY = 'pos-cart';

export const CartProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [cart, setCart] = useState<CartItem[]>([]);

  useEffect(() => {
    const saved = localStorage.getItem(CART_STORAGE_KEY);
    if (!saved) return;
    try {
      const items: CartItem[] = JSON.parse(saved);
      setCart(items);
    } catch {
      localStorage.removeItem(CART_STORAGE_KEY);
    }
  }, []);

  useEffect(() => {
    localStorage.setItem(CART_STORAGE_KEY, JSON.stringify(cart));
  }, [cart]);

  const addItem = (product: Product) => {
    setCart(prev => {
      const existing = prev.find(item => item.id === product.id);
      if (existing) {
        return prev.map(item =>
          item.id === product.id ? { ...item, quantity: item.quantity + 1 } : item
        );
      }
      return [...prev, { ...product, quantity: 1 }];
    });
  };

  const incrementItemQuantity = (productId: string) => {
    setCart(prev => prev.map(item =>
      item.id === productId ? { ...item, quantity: item.quantity + 1 } : item
    ));
  };

  const decrementItemQuantity = (productId: string) => {
    setCart(prev => prev.map(item =>
      item.id === productId && item.quantity > 1
        ? { ...item, quantity: item.quantity - 1 }
        : item
    ));
  };

  const removeItem = (productId: string) => {
    setCart(prev => prev.filter(item => item.id !== productId));
  };

  const clearCart = () => {
    setCart([]);
  };

  const updateItemPrice = (productId: string, price: number) => {
    setCart(prev => prev.map(item =>
      item.id === productId ? { ...item, price } : item
    ));
  };

  const totalAmount = cart.reduce((sum, item) => sum + item.price * item.quantity, 0);

  return (
    <CartContext.Provider
      value={{
        cart,
        addItem,
        removeItem,
        clearCart,
        incrementItemQuantity,
        decrementItemQuantity,
        updateItemPrice,
        totalAmount,
      }}
    >
      {children}
    </CartContext.Provider>
  );
};

export const useCart = () => {
  const ctx = useContext(CartContext);
  if (!ctx) throw new Error('useCart must be used within CartProvider');
  return ctx;
};
