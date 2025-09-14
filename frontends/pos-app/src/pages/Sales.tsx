// src/pages/Sales.tsx
import React, { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import { useCart } from '../CartContext';
import { simulatePayment } from '../utils/payments';

// Sample hardcoded product catalog
const PRODUCTS = [
  { id: 1, name: 'Widget A', price: 19.99 },
  { id: 2, name: 'Widget B', price: 5.49 },
  { id: 3, name: 'Gadget C', price: 12.00 },
];

const Sales: React.FC = () => {
  const { logout } = useAuth();
  const { cart, addItem, removeItem, clearCart, totalAmount } = useCart();
  const navigate = useNavigate();

  // If not logged in (no auth), redirect to login – alternatively, we could use <RequireAuth> at routing level.
  // (This effect is optional since we guard route with RequireAuth already)
  const { isLoggedIn } = useAuth();
  useEffect(() => {
    if (!isLoggedIn) navigate('/');
  }, [isLoggedIn, navigate]);

  // Handler for payment simulation
  const handlePayment = async (method: 'card' | 'cash' | 'crypto') => {
    alert(`Processing ${method.toUpperCase()} payment...`);
    try {
      await simulatePayment(method, totalAmount);
      alert(`${method.charAt(0).toUpperCase() + method.slice(1)} payment successful!`);
      clearCart();
    } catch (err) {
      alert(`Payment failed: ${err}`);
    }
  };

  return (
    <div className="sales-screen">
      <header>
        <h2>Sales Terminal</h2>
        <button onClick={() => { logout(); navigate('/'); }}>
          Logout
        </button>
      </header>

      <div className="content">
        {/* Products List */}
        <div className="products-section">
          <h3>Products</h3>
          <ul>
            {PRODUCTS.map(product => (
              <li key={product.id}>
                {product.name} – ${product.price.toFixed(2)}
                <button onClick={() => addItem(product)}>Add</button>
              </li>
            ))}
          </ul>
        </div>

        {/* Cart Section */}
        <div className="cart-section">
          <h3>Cart</h3>
          {cart.length === 0 ? (
            <p>Your cart is empty.</p>
          ) : (
            <ul>
              {cart.map(item => (
                <li key={item.id}>
                  {item.name} x {item.quantity} = ${ (item.price * item.quantity).toFixed(2) }
                  <button onClick={() => removeItem(item.id)} style={{ marginLeft: '1em' }}>
                    Remove
                  </button>
                </li>
              ))}
            </ul>
          )}
          <p><strong>Total: ${totalAmount.toFixed(2)}</strong></p>

          {/* Checkout/Payment */}
          {cart.length > 0 && (
            <div className="checkout">
              <h4>Choose Payment Method:</h4>
              <button onClick={() => handlePayment('card')}>Pay with Card</button>
              <button onClick={() => handlePayment('cash')}>Pay with Cash</button>
              <button onClick={() => handlePayment('crypto')}>Pay with Crypto</button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default Sales;
