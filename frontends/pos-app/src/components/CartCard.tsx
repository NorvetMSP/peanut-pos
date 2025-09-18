import React from 'react';
import './CartCard.css';

type CartItem = {
  id: string;
  name: string;
  price: number;
  quantity: number;
  onRemove: () => void;
};

type CartCardProps = {
  items: CartItem[];
  total: number;
  onCheckout: () => void;
};

const CartCard: React.FC<CartCardProps> = ({ items, total, onCheckout }) => (
  <aside className="w-full max-w-md mx-auto bg-white dark:bg-gray-900 rounded-xl shadow-lg p-6 flex flex-col">
    <h2 className="text-xl font-bold mb-4 text-gray-900 dark:text-gray-100 tracking-tight">Cart</h2>
    <div className="flex-1 overflow-y-auto">
      {items.length === 0 ? (
        <p className="text-gray-500 dark:text-gray-400">Your cart is empty.</p>
      ) : (
        <ul className="space-y-2">
          {items.map(item => (
            <li key={item.id} className="flex items-center justify-between bg-gray-50 rounded-lg px-3 py-2 shadow-sm">
              <div className="flex flex-col">
                <span className="font-semibold text-gray-900 text-sm">{item.name}</span>
                <span className="text-xs text-gray-500">Qty: {item.quantity}</span>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-base font-bold text-gray-800">${(item.price * item.quantity).toFixed(2)}</span>
                <button className="cart-remove-modern" onClick={item.onRemove}>Remove</button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
    <div className="mt-6 flex items-center justify-between text-lg font-bold text-gray-900 dark:text-gray-100">
      <span>Total:</span>
      <span>${total.toFixed(2)}</span>
    </div>
    {items.length > 0 && (
      <button
        className="cart-proceed-btn"
        onClick={onCheckout}
      >
        Proceed to Checkout
      </button>
    )}
  </aside>
);

export default CartCard;
