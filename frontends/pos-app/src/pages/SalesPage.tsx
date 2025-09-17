import React, { useEffect } from 'react';
import logoTransparent from '../assets/logo_transparent.png';
import productIcon from '../assets/react.svg';
import ProductCardModern from '../components/ProductCardModern';
import '../components/ProductCardModern.css';
import CartCard from '../components/CartCard';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import { useCart } from '../CartContext';
import { simulatePayment } from '../utils/payments';

const PRODUCTS = [
  { id: 1, name: 'Widget A', price: 19.99 },
  { id: 2, name: 'Widget B', price: 5.49 },
  { id: 3, name: 'Gadget C', price: 12.00 },
];

const SalesPage: React.FC = () => {
  const { logout, isLoggedIn } = useAuth();
  const { cart, addItem, removeItem, clearCart, incrementItemQuantity, decrementItemQuantity, totalAmount } = useCart();

  // Helper functions for quantity adjustment
  const incrementQty = (item: { id: number }) => {
    incrementItemQuantity(item.id);
  };
  const decrementQty = (item: { id: number; quantity: number }) => {
    if (item.quantity > 1) {
      decrementItemQuantity(item.id);
    }
  };
  const navigate = useNavigate();

  useEffect(() => {
    if (!isLoggedIn) navigate('/');
  }, [isLoggedIn, navigate]);

  const handlePayment = async (method: 'card' | 'cash' | 'crypto') => {
    alert(`Processing ${method.toUpperCase()} payment...`);
    try {
      await simulatePayment(method, totalAmount);
      alert(`${method.charAt(0).toUpperCase() + method.slice(1)} payment successful!`);
      clearCart();
      navigate('/checkout');
    } catch (err) {
      alert(`Payment failed: ${err}`);
    }
  };

  return (
    <div className="min-h-screen bg-gradient-to-br from-gray-100 to-gray-300 dark:from-gray-900 dark:to-gray-800 flex flex-col">
      {/* Header with logo and logout */}
      <header className="flex items-center justify-between px-6 py-4 bg-white dark:bg-gray-900 shadow-md">
        <div className="flex items-center gap-3">
          <img src={logoTransparent} alt="NovaPOS Logo" className="h-10 w-auto" />
          <span className="text-2xl font-bold text-primary dark:text-white tracking-tight">NovaPOS</span>
        </div>
        <button className="bg-red-500 text-white px-4 py-2 rounded hover:bg-red-600" onClick={() => { logout(); navigate('/'); }}>Logout</button>
      </header>

      {/* Centered Main Content */}
      <div className="flex-1 flex items-center justify-center w-full">
        <main className="flex flex-col gap-8 px-4 py-8 items-center justify-center w-full max-w-4xl mx-auto">
          {/* Products Section */}
          <section className="w-full max-w-2xl mx-auto">
            <h2 className="text-xl font-semibold mb-6 text-gray-800 dark:text-gray-100">Products</h2>
            <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-6">
              {PRODUCTS.map(product => (
                <ProductCardModern
                  key={product.id}
                  product={{
                    ...product,
                    description: 'A great product for your POS needs.',
                    image: productIcon,
                    onAdd: () => addItem(product),
                    onWishlist: () => alert(`Added ${product.name} to wishlist!`)
                  }}
                />
              ))}
            </div>
          </section>

          {/* Cart Section - centered below products */}
          <CartCard
            items={cart.map(item => ({
              ...item,
              onRemove: () => removeItem(item.id),
              onAdd: () => incrementQty(item),
              onSubtract: () => decrementQty(item)
            }))}
            total={totalAmount}
            onCheckout={() => handlePayment('card')}
          />
        </main>
      </div>
    </div>
  );
};

export default SalesPage;





