import React, { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import './AdminSectionModern.css';

interface Product { name: string; price: number; }

const ProductListPage: React.FC = () => {
  const { isLoggedIn } = useAuth();
  const navigate = useNavigate();
  // Dummy product list state
  const [products, setProducts] = useState<Product[]>([
    { name: 'Sample Product A', price: 29.99 },
    { name: 'Sample Product B', price: 15.5 }
  ]);
  // Form state for new product
  const [newProduct, setNewProduct] = useState<Product>({ name: '', price: 0 });

  // Protect route: redirect to login if not auth
  useEffect(() => {
    if (!isLoggedIn) navigate('/');
  }, [isLoggedIn, navigate]);

  const handleAddProduct = (e: React.FormEvent) => {
    e.preventDefault();
    if (!newProduct.name.trim()) return;  // require a name (basic validation)
    // Add product to list (stubbed – no server call)
    setProducts(prev => [...prev, newProduct]);
    // Reset the form fields
    setNewProduct({ name: '', price: 0 });
  };

  return (
    <div className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col" style={{ fontFamily: 'Raleway, sans-serif', background: 'linear-gradient(135deg, #f8fafc 0%, #e6f7fa 100%)' }}>
      <div className="admin-section-modern">
        <div className="admin-section-header">
          <h2>Products</h2>
          <p>Manage your product catalog and pricing.</p>
        </div>
        <div className="admin-section-content">
        <h2 className="text-2xl font-semibold mb-4 text-gray-800 dark:text-gray-100">Product Management</h2>
        {/* Form to add a new product */}
        <form onSubmit={handleAddProduct} className="mb-6 p-4 bg-white dark:bg-gray-800 rounded shadow max-w-lg">
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
            <div>
              <label className="block text-gray-700 dark:text-gray-200 mb-1">Product Name</label>
              <input 
                type="text" 
                value={newProduct.name}
                onChange={e => setNewProduct({ ...newProduct, name: e.target.value })}
                className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 
                           focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                placeholder="Name" 
                required 
              />
            </div>
            <div>
              <label className="block text-gray-700 dark:text-gray-200 mb-1">Price ($)</label>
              <input 
                type="number" step="0.01"
                value={newProduct.price !== 0 ? newProduct.price : ''}  // show blank if 0
                onChange={e => setNewProduct({ ...newProduct, price: parseFloat(e.target.value) || 0 })}
                className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 
                           focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                placeholder="Price" 
                required 
                min="0"
              />
            </div>
          </div>
          <button 
            type="submit" 
            className="mt-4 px-4 py-2 rounded-md text-white"
            style={{ background: '#19b4b9' }}
            onMouseOver={e => (e.currentTarget.style.background = '#153a5b')}
            onMouseOut={e => (e.currentTarget.style.background = '#19b4b9')}
          >
            Add Product
          </button>
        </form>

        {/* Products list table */}
        <div className="overflow-x-auto max-w-lg">
          <table className="min-w-full bg-white dark:bg-gray-800 rounded shadow">
            <thead className="bg-gray-200 dark:bg-gray-700">
              <tr>
                <th className="text-left px-4 py-2 text-gray-800 dark:text-gray-100">Product Name</th>
                <th className="text-left px-4 py-2 text-gray-800 dark:text-gray-100">Price ($)</th>
              </tr>
            </thead>
            <tbody>
              {products.map((prod, idx) => (
                <tr key={idx} className="border-b border-gray-200 dark:border-gray-700">
                  <td className="px-4 py-2 text-gray-900 dark:text-gray-100">{prod.name}</td>
                  <td className="px-4 py-2 text-gray-900 dark:text-gray-100">${prod.price.toFixed(2)}</td>
                </tr>
              ))}
              {products.length === 0 && (
                <tr>
                  <td colSpan={2} className="px-4 py-2 text-center text-gray-500">No products available.</td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        </div>
        <div style={{ textAlign: 'right', marginTop: '2rem' }}>
          <button className="admin-section-btn" onClick={() => navigate('/home')}>Back to Admin Home</button>
        </div>
      </div>
    </div>
  );
};

export default ProductListPage;
