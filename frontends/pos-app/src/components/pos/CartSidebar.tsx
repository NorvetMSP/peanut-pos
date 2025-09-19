import React from 'react';
import type { CartItem } from '../../CartContext';

type CartSidebarProps = {
  items: CartItem[];
  inactiveItemIds?: string[];
  onAddQty: (productId: string) => void;
  onSubQty: (productId: string) => void;
  onRemoveItem: (productId: string) => void;
};

const formatCurrency = (value: number): string => `$${value.toFixed(2)}`;

const CartSidebar: React.FC<CartSidebarProps> = ({ items, inactiveItemIds = [], onAddQty, onSubQty, onRemoveItem }) => {
  const total = items.reduce((sum, item) => sum + item.price * item.quantity, 0);
  const itemCount = items.reduce((sum, item) => sum + item.quantity, 0);

  return (
    <div className="cashier-cart-panel">
      <div className="cashier-cart-panel__header">
        <h2>Cart</h2>
        <span>{itemCount} item{itemCount === 1 ? '' : 's'}</span>
      </div>
      <div className="cashier-cart-items">
        {items.length === 0 ? (
          <p className="cashier-cart-panel__empty">
            Your cart is empty. Add items from the catalog to begin a sale.
          </p>
        ) : (
          items.map(item => {
            const isInactive = inactiveItemIds.includes(item.id);
            const lineTotal = item.price * item.quantity;
            return (
              <div key={item.id} className="cashier-cart-item">
                <div className="cashier-cart-item__top">
                  <div className="cashier-cart-item__info">
                    <div className="cashier-cart-item__title">{item.name}</div>
                    <div className="cashier-cart-item__subtitle">Qty {item.quantity} × {formatCurrency(item.price)}</div>
                    {isInactive && <div className="cashier-inactive-pill">⚠ Item inactive — replace or remove</div>}
                  </div>
                  <button
                    type="button"
                    className="cashier-cart-item__delete"
                    onClick={() => onRemoveItem(item.id)}
                    title="Remove item"
                  >
                    ×
                  </button>
                </div>
                <div className="cashier-cart-item__bottom">
                  <div className="cashier-cart-value">{formatCurrency(lineTotal)}</div>
                  <div className="cashier-cart-qty">
                    <button
                      type="button"
                      onClick={() => onSubQty(item.id)}
                      disabled={item.quantity <= 1}
                      aria-label="Decrease quantity"
                    >
                      –
                    </button>
                    <span>{item.quantity}</span>
                    <button
                      type="button"
                      onClick={() => onAddQty(item.id)}
                      aria-label="Increase quantity"
                    >
                      +
                    </button>
                  </div>
                </div>
              </div>
            );
          })
        )}
      </div>
      <div className="cashier-cart-total">
        <span>Total</span>
        <span>{formatCurrency(total)}</span>
      </div>
    </div>
  );
};

export default CartSidebar;
