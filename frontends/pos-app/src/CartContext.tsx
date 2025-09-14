// src/CartContext.tsx
import React, { createContext, useContext, useEffect, useState } from 'react';

// Define product and cart item types
interface Product { id: number; name: string; price: number; }
interface CartItem extends Product { quantity: number; }

interface CartContextValue {
  cart: CartItem[];
  addItem: (product: Product) => void;
  removeItem: (productId: number) => void;
  clearCart: () => void;
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
      // Check if product already in cart
      const existing = prev.find(item => item.id === product.id);
      if (existing) {
        // Increment quantity if exists
        return prev.map(item =>
          item.id === product.id ? { ...item, quantity: item.quantity + 1 } : item
        );
      } else {
        // Add new item
        return [...prev, { ...product, quantity: 1 }];
      }
    });
  };

  const removeItem = (productId: number) => {
    setCart(prev => prev.filter(item => item.id !== productId));
  };

  const clearCart = () => {
    setCart([]);
  };

  // Compute total amount
  const totalAmount = cart.reduce((sum, item) => sum + item.price * item.quantity, 0);

  return (
    <CartContext.Provider value={{ cart, addItem, removeItem, clearCart, totalAmount }}>
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
