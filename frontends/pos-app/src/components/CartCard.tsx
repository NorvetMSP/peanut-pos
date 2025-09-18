import React from 'react';
import './CartCard.css';

type CartItemStatus = {
  inactive: boolean;
  priceChanged: boolean;
  latestPrice?: number;
};

type CartItem = {
  id: string;
  name: string;
  price: number;
  quantity: number;
  onRemove: () => void;
  onIncrement: () => void;
  onDecrement: () => void;
  status?: CartItemStatus;
};

type CartCardProps = {
  items: CartItem[];
  subtotal: number;
  tax: number;
  total: number;
  taxRate: number;
  onCheckout: () => void;
  onClear: () => void;
};

const CartCard: React.FC<CartCardProps> = ({ items, subtotal, tax, total, taxRate, onCheckout, onClear }) => {
  const taxLabel = `${(taxRate * 100).toFixed(1)}%`;

  return (
    <aside className="w-full max-w-md mx-auto bg-white dark:bg-gray-900 rounded-xl shadow-lg p-6 flex flex-col" aria-label="Shopping cart">
      <div className="cart-header-row">
        <h2 className="text-xl font-bold text-gray-900 dark:text-gray-100 tracking-tight">Cart</h2>
        {items.length > 0 && (
          <button type="button" className="cart-clear-btn" onClick={onClear} aria-label="Clear cart">
            Clear
          </button>
        )}
      </div>
      <div className="flex-1 overflow-y-auto" role="list">
        {items.length === 0 ? (
          <p className="text-gray-500 dark:text-gray-400">Your cart is empty.</p>
        ) : (
          <ul className="space-y-3">
            {items.map(item => {
              const { inactive, priceChanged, latestPrice } = item.status ?? { inactive: false, priceChanged: false };
              const lockedPrice = item.price.toFixed(2);
              return (
                <li key={item.id} className="cart-item-row" role="listitem">
                  <div className="cart-item-info">
                    <span className="cart-item-name">{item.name}</span>
                    <div className="cart-item-meta">
                      <div className="cart-qty-gadget" aria-label={`Quantity controls for ${item.name}`}>
                        <button
                          type="button"
                          className="cart-qty-btn"
                          onClick={item.onDecrement}
                          disabled={item.quantity <= 1}
                          aria-label={`Decrease quantity of ${item.name}`}
                        >
                          -
                        </button>
                        <span className="cart-qty-value" aria-live="polite">{item.quantity}</span>
                        <button
                          type="button"
                          className="cart-qty-btn"
                          onClick={item.onIncrement}
                          aria-label={`Increase quantity of ${item.name}`}
                        >
                          +
                        </button>
                      </div>
                      <span className="cart-price-lock">Locked at ${lockedPrice}</span>
                      {inactive && <span className="cart-status-badge cart-status-badge--inactive">Inactive in catalog</span>}
                      {priceChanged && (
                        <span className="cart-status-badge cart-status-badge--price">
                          Catalog now ${latestPrice !== undefined ? latestPrice.toFixed(2) : '—'}
                        </span>
                      )}
                    </div>
                  </div>
                  <div className="cart-item-actions">
                    <span className="cart-line-total" aria-label={`Line total for ${item.name}`}>
                    {'$' + (item.price * item.quantity).toFixed(2)}
                    </span>
                    <button type="button" className="cart-remove-modern" onClick={item.onRemove} aria-label={`Remove ${item.name}`}>
                      Remove
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>
      <div className="cart-summary" aria-live="polite">
        <div className="cart-summary-row">
          <span>Subtotal</span>
          <span>${subtotal.toFixed(2)}</span>
        </div>
        <div className="cart-summary-row">
          <span>Tax ({taxLabel})</span>
          <span>${tax.toFixed(2)}</span>
        </div>
        <div className="cart-summary-row cart-summary-total">
          <span>Total</span>
          <span>${total.toFixed(2)}</span>
        </div>
      </div>
      {items.length > 0 && (
        <button className="cart-proceed-btn" onClick={onCheckout} aria-label="Proceed to checkout">
          Proceed to Checkout
        </button>
      )}
    </aside>
  );
};

export default CartCard;

