import React from 'react';
import type { CartItem } from '../../CartContext';

type CartSidebarProps = {
  items: CartItem[];
  inactiveItemIds?: string[];
  onAddQty: (productId: string) => void;
  onSubQty: (productId: string) => void;
  onRemoveItem: (productId: string) => void;
};

const CartSidebar: React.FC<CartSidebarProps> = ({ items, inactiveItemIds = [], onAddQty, onSubQty, onRemoveItem }) => {
  const total = items.reduce((sum, item) => sum + item.price * item.quantity, 0);

  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-4 flex flex-col max-h-[60vh]">
      <h2 className="text-xl font-bold mb-3 text-gray-800 dark:text-gray-100">Cart</h2>
      <div className="flex-1 overflow-y-auto pr-1">
        {items.length === 0 ? (
          <p className="text-gray-500 dark:text-gray-400">Your cart is empty.</p>
        ) : (
          <ul className="space-y-2">
            {items.map(item => {
              const isInactive = inactiveItemIds.includes(item.id);
              return (
                <li key={item.id} className="flex items-center justify-between bg-gray-50 dark:bg-gray-700 rounded px-3 py-2">
                  <div className="flex-1 mr-2">
                    <div className="text-sm font-semibold text-gray-800 dark:text-gray-100">
                      {item.name}
                      {isInactive && <span className="text-red-600 text-xs ml-1">(inactive)</span>}
                    </div>
                    <div className="text-xs text-gray-500 dark:text-gray-400">
                      Qty {item.quantity} × ${item.price.toFixed(2)}
                    </div>
                  </div>
                  <div className="flex items-center">
                    <button
                      type="button"
                      className="px-2 text-lg text-gray-700 dark:text-gray-200 disabled:opacity-50"
                      onClick={() => onSubQty(item.id)}
                      disabled={item.quantity <= 1}
                      aria-label="Decrease quantity"
                    >
                      &minus;
                    </button>
                    <span className="px-2 text-sm text-gray-800 dark:text-gray-100">{item.quantity}</span>
                    <button
                      type="button"
                      className="px-2 text-lg text-gray-700 dark:text-gray-200"
                      onClick={() => onAddQty(item.id)}
                      aria-label="Increase quantity"
                    >
                      +
                    </button>
                    <button
                      type="button"
                      className="ml-3 text-red-600 dark:text-red-400 text-xl"
                      onClick={() => onRemoveItem(item.id)}
                      title="Remove item"
                    >
                      &times;
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>
      <div className="mt-3 pt-3 border-t border-gray-200 dark:border-gray-700 text-lg font-semibold flex justify-between text-gray-800 dark:text-gray-100">
        <span>Total:</span>
        <span>${total.toFixed(2)}</span>
      </div>
    </div>
  );
};

export default CartSidebar;
