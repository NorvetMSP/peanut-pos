// src/CartContext.tsx
import React, { createContext, useContext, useEffect, useState } from 'react';

// Define product and cart item types aligned with service payloads
interface Product { id: string; name: string; price: number; sku?: string | null; }
interface CartItem extends Product { quantity: number; }

interface CartContextValue {
  cart: CartItem[];
  addItem: (product: Product) => void;
  removeItem: (productId: string) => void;
  clearCart: () => void;
  incrementItemQuantity: (productId: string) => void;
  decrementItemQuantity: (productId: string) => void;
  totalAmount: number;
}
const CartContext = createContext<CartContextValue | undefined>(undefined);

export const CartProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [cart, setCart] = useState<CartItem[]>([]);

  // Load cart from local storage on init (for offline persistence)
  useEffect(() => {
    const saved = localStorage.getItem('pos-cart');
    if (saved) {
      try {
        const items: CartItem[] = JSON.parse(saved);
        setCart(items);
      } catch {}
    }
  }, []);

  // Save cart to local storage whenever it changes
  useEffect(() => {
    localStorage.setItem('pos-cart', JSON.stringify(cart));
  }, [cart]);

  const addItem = (product: Product) => {
    setCart(prev => {
      const existing = prev.find(item => item.id === product.id);
      if (existing) {
        return prev.map(item =>
          item.id === product.id ? { ...item, quantity: item.quantity + 1 } : item
        );
      } else {
        return [...prev, { ...product, quantity: 1 }];
      }
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

  // Compute total amount
  const totalAmount = cart.reduce((sum, item) => sum + item.price * item.quantity, 0);

  return (
    <CartContext.Provider value={{ cart, addItem, removeItem, clearCart, incrementItemQuantity, decrementItemQuantity, totalAmount }}>
      {children}
    </CartContext.Provider>
  );
};

// Hook to use cart context
export const useCart = () => {
  const ctx = useContext(CartContext);
  if (!ctx) throw new Error('useCart must be used within CartProvider');
  return ctx;
};

