import React, { useEffect } from 'react';
import logoTransparent from '../../../assets/logo/logo_transparent.png';
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
  const { cart, addItem, removeItem, clearCart, totalAmount } = useCart();
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

      {/* Main content: Products and Cart */}
  <main className="flex flex-col md:flex-row flex-1 gap-8 px-4 py-8 items-center justify-center w-full">
        {/* Products Section */}
  <section className="md:w-2/3 w-full max-w-2xl mx-auto">
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

        {/* Cart Section */}
  <aside className="md:w-1/3 w-full max-w-md mx-auto bg-white dark:bg-gray-900 rounded-xl shadow-lg p-6 flex flex-col mt-8 md:mt-0">
          <h2 className="text-xl font-semibold mb-4 text-gray-800 dark:text-gray-100">Cart</h2>
          <div className="flex-1 space-y-3 overflow-y-auto">
            {cart.length === 0 ? (
              <p className="text-gray-500 dark:text-gray-400">Your cart is empty.</p>
            ) : (
              cart.map(item => (
                <CartCard
                  key={item.id}
                  item={{
                    ...item,
                    onRemove: () => removeItem(item.id)
                  }}
                />
              ))
            )}
          </div>
          <div className="mt-6 text-right text-gray-800 dark:text-gray-100 text-lg">
            <span className="font-bold">Total: ${totalAmount.toFixed(2)}</span>
          </div>
          {cart.length > 0 && (
            <div className="mt-6">
              <h4 className="mb-3 text-gray-700 dark:text-gray-200 text-base">Choose Payment Method:</h4>
              <div className="flex flex-col gap-3">
                <button className="bg-primary text-white py-2 px-4 rounded-lg hover:bg-opacity-90" onClick={() => handlePayment('card')}>Pay with Card</button>
                <button className="bg-primary text-white py-2 px-4 rounded-lg hover:bg-opacity-90" onClick={() => handlePayment('cash')}>Pay with Cash</button>
                <button className="bg-primary text-white py-2 px-4 rounded-lg hover:bg-opacity-90" onClick={() => handlePayment('crypto')}>Pay with Crypto</button>
              </div>
            </div>
          )}
        </aside>
      </main>
    </div>
  );
};

export default SalesPage;
