import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import './AdminSectionModern.css';

const PRODUCT_SERVICE_URL = (import.meta.env.VITE_PRODUCT_SERVICE_URL ?? 'http://localhost:8081').replace(/\/$/, '');

type ServiceProduct = {
  id: string;
  name: string;
  price: number;
  description: string;
  active: boolean;
};

type ProductFormState = {
  name: string;
  price: string;
  description: string;
};

type EditFormState = ProductFormState & { active: boolean };

const normalizeProduct = (input: unknown): ServiceProduct | null => {
  if (!input || typeof input !== 'object') return null;
  const candidate = input as Record<string, unknown>;
  const id = candidate.id;
  const name = candidate.name;
  const priceRaw = candidate.price;
  if (typeof id !== 'string' || typeof name !== 'string') return null;
  const price = typeof priceRaw === 'number' ? priceRaw : Number(priceRaw);
  if (!Number.isFinite(price)) return null;
  return {
    id,
    name,
    price,
    description: typeof candidate.description === 'string' ? candidate.description : '',
    active: typeof candidate.active === 'boolean' ? candidate.active : true,
  };
};

const ProductListPage: React.FC = () => {
  const { isLoggedIn, currentUser, token } = useAuth();
  const navigate = useNavigate();

  const [products, setProducts] = useState<ServiceProduct[]>([]);
  const [newProduct, setNewProduct] = useState<ProductFormState>({ name: '', price: '', description: '' });
  const [editingProductId, setEditingProductId] = useState<string | null>(null);
  const [editForm, setEditForm] = useState<EditFormState>({ name: '', price: '', description: '', active: true });
  const [isLoading, setIsLoading] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [updatingProductId, setUpdatingProductId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  useEffect(() => {
    if (!isLoggedIn) navigate('/login', { replace: true });
  }, [isLoggedIn, navigate]);

  const tenantId = currentUser?.tenant_id ? String(currentUser.tenant_id) : null;

  const buildHeaders = useCallback((): Record<string, string> => {
    const headers: Record<string, string> = {};
    if (tenantId) headers['X-Tenant-ID'] = tenantId;
    if (token) headers['Authorization'] = `Bearer ${token}`;
    return headers;
  }, [tenantId, token]);

  const ensureTenantContext = useCallback((): boolean => {
    if (!tenantId) {
      setError('Tenant context is unavailable. Please log out and back in.');
      setProducts([]);
      return false;
    }
    return true;
  }, [tenantId]);

  const fetchProducts = useCallback(async () => {
    if (!ensureTenantContext()) return;
    setIsLoading(true);
    setError(null);
    try {
      const response = await fetch(`${PRODUCT_SERVICE_URL}/products`, {
        headers: buildHeaders(),
      });
      if (!response.ok) {
        throw new Error(`Failed to fetch products (${response.status})`);
      }
      const data = await response.json();
      const normalized = Array.isArray(data)
        ? data.map(normalizeProduct).filter((item): item is ServiceProduct => Boolean(item))
        : [];
      setProducts(normalized);
    } catch (err) {
      console.error('Unable to load products', err);
      setError('Unable to load products. Please try again.');
    } finally {
      setIsLoading(false);
    }
  }, [buildHeaders, ensureTenantContext]);

  useEffect(() => {
    fetchProducts();
  }, [fetchProducts]);

  const resetEditState = () => {
    setEditingProductId(null);
    setEditForm({ name: '', price: '', description: '', active: true });
  };

  const handleAddProduct = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setSuccessMessage(null);

    if (!ensureTenantContext()) return;

    const trimmedName = newProduct.name.trim();
    const priceValue = Number(newProduct.price);
    if (!trimmedName) {
      setError('Product name is required.');
      return;
    }
    if (!Number.isFinite(priceValue) || priceValue < 0) {
      setError('Enter a valid price.');
      return;
    }

    setIsSubmitting(true);
    try {
      const response = await fetch(`${PRODUCT_SERVICE_URL}/products`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...buildHeaders(),
        },
        body: JSON.stringify({
          name: trimmedName,
          price: priceValue,
          description: newProduct.description.trim(),
        }),
      });
      if (!response.ok) {
        throw new Error(`Failed to add product (${response.status})`);
      }
      const created = normalizeProduct(await response.json());
      if (created) {
        setProducts(prev => [...prev, created]);
      }
      setNewProduct({ name: '', price: '', description: '' });
      setSuccessMessage('Product added successfully.');
    } catch (err) {
      console.error('Unable to add product', err);
      setError('Unable to add product. Please try again.');
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleStartEdit = (product: ServiceProduct) => {
    setError(null);
    setSuccessMessage(null);
    setEditingProductId(product.id);
    setEditForm({
      name: product.name,
      price: product.price.toFixed(2),
      description: product.description,
      active: product.active,
    });
  };

  const handleSaveEdit = async () => {
    if (!editingProductId) return;
    if (!ensureTenantContext()) return;

    const trimmedName = editForm.name.trim();
    const priceValue = Number(editForm.price);
    if (!trimmedName) {
      setError('Product name is required.');
      return;
    }
    if (!Number.isFinite(priceValue) || priceValue < 0) {
      setError('Enter a valid price.');
      return;
    }

    setError(null);
    setSuccessMessage(null);
    setUpdatingProductId(editingProductId);
    try {
      const response = await fetch(`${PRODUCT_SERVICE_URL}/products/${editingProductId}`, {
        method: 'PUT',
        headers: {
          'Content-Type': 'application/json',
          ...buildHeaders(),
        },
        body: JSON.stringify({
          name: trimmedName,
          price: priceValue,
          description: editForm.description.trim(),
          active: editForm.active,
        }),
      });
      if (!response.ok) {
        throw new Error(`Failed to update product (${response.status})`);
      }
      const updated = normalizeProduct(await response.json());
      if (updated) {
        setProducts(prev => prev.map(prod => (prod.id === updated.id ? updated : prod)));
      }
      setSuccessMessage('Product updated successfully.');
      resetEditState();
    } catch (err) {
      console.error('Unable to update product', err);
      setError('Unable to update product. Please try again.');
    } finally {
      setUpdatingProductId(null);
    }
  };

  const handleToggleActive = async (product: ServiceProduct) => {
    if (!ensureTenantContext()) return;
    setError(null);
    setSuccessMessage(null);
    setUpdatingProductId(product.id);
    try {
      const response = await fetch(`${PRODUCT_SERVICE_URL}/products/${product.id}`, {
        method: 'PUT',
        headers: {
          'Content-Type': 'application/json',
          ...buildHeaders(),
        },
        body: JSON.stringify({
          name: product.name,
          price: product.price,
          description: product.description,
          active: !product.active,
        }),
      });
      if (!response.ok) {
        throw new Error(`Failed to update product (${response.status})`);
      }
      const updated = normalizeProduct(await response.json());
      if (updated) {
        setProducts(prev => prev.map(prod => (prod.id === updated.id ? updated : prod)));
      }
      setSuccessMessage(`Product ${product.active ? 'deactivated' : 'activated'} successfully.`);
    } catch (err) {
      console.error('Unable to toggle product status', err);
      setError('Unable to update product status. Please try again.');
    } finally {
      setUpdatingProductId(null);
    }
  };

  const handleRefresh = () => {
    setSuccessMessage(null);
    setError(null);
    fetchProducts();
  };

  const sortedProducts = useMemo(() => [...products].sort((a, b) => a.name.localeCompare(b.name)), [products]);

  return (
    <div className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col" style={{ fontFamily: 'Raleway, sans-serif', background: 'linear-gradient(135deg, #f8fafc 0%, #e6f7fa 100%)' }}>
      <div className="admin-section-modern">
        <div className="admin-section-header">
          <h2>Products</h2>
          <p>Manage your product catalog and pricing.</p>
        </div>
        <div className="admin-section-content">
          <h2 className="text-2xl font-semibold mb-4 text-gray-800 dark:text-gray-100">Product Management</h2>
          {error && <div className="mb-4 rounded bg-red-100 text-red-700 px-4 py-2">{error}</div>}
          {successMessage && <div className="mb-4 rounded bg-emerald-100 text-emerald-700 px-4 py-2">{successMessage}</div>}
          {!tenantId && (
            <div className="mb-4 rounded bg-yellow-100 text-yellow-700 px-4 py-2">
              Tenant information is missing. Log out and sign back in as a tenant user.
            </div>
          )}
          <form onSubmit={handleAddProduct} className="mb-6 p-4 bg-white dark:bg-gray-800 rounded shadow max-w-lg">
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
              <div>
                <label className="block text-gray-700 dark:text-gray-200 mb-1" htmlFor="product-name">Product Name</label>
                <input
                  id="product-name"
                  type="text"
                  value={newProduct.name}
                  onChange={e => setNewProduct({ ...newProduct, name: e.target.value })}
                  className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                  placeholder="Name"
                  required
                  disabled={isSubmitting}
                />
              </div>
              <div>
                <label className="block text-gray-700 dark:text-gray-200 mb-1" htmlFor="product-price">Price ($)</label>
                <input
                  id="product-price"
                  type="number"
                  step="0.01"
                  value={newProduct.price}
                  onChange={e => setNewProduct({ ...newProduct, price: e.target.value })}
                  className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                  placeholder="Price"
                  required
                  min="0"
                  disabled={isSubmitting}
                />
              </div>
            </div>
            <div className="mt-4">
              <label className="block text-gray-700 dark:text-gray-200 mb-1" htmlFor="product-description">Description</label>
              <textarea
                id="product-description"
                value={newProduct.description}
                onChange={e => setNewProduct({ ...newProduct, description: e.target.value })}
                className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                placeholder="Short description"
                rows={3}
                disabled={isSubmitting}
              />
            </div>
            <button
              type="submit"
              className="mt-4 px-4 py-2 rounded-md text-white"
              style={{ background: '#19b4b9' }}
              onMouseOver={e => (e.currentTarget.style.background = '#153a5b')}
              onMouseOut={e => (e.currentTarget.style.background = '#19b4b9')}
              disabled={isSubmitting || !tenantId}
            >
              {isSubmitting ? 'Adding...' : 'Add Product'}
            </button>
          </form>

          <div className="overflow-x-auto max-w-4xl">
            <div className="flex items-center justify-between mb-2">
              <span className="text-sm text-gray-600 dark:text-gray-300">{isLoading ? 'Loading products...' : `${products.length} product${products.length === 1 ? '' : 's'}`}</span>
              <button
                type="button"
                className="text-sm text-primary hover:underline"
                onClick={handleRefresh}
                disabled={isLoading}
              >
                Refresh
              </button>
            </div>
            <table className="min-w-full bg-white dark:bg-gray-800 rounded shadow">
              <thead className="bg-gray-200 dark:bg-gray-700">
                <tr>
                  <th className="text-left px-4 py-2 text-gray-800 dark:text-gray-100">Product Name</th>
                  <th className="text-left px-4 py-2 text-gray-800 dark:text-gray-100">Price ($)</th>
                  <th className="text-left px-4 py-2 text-gray-800 dark:text-gray-100">Status</th>
                  <th className="text-left px-4 py-2 text-gray-800 dark:text-gray-100">Actions</th>
                </tr>
              </thead>
              <tbody>
                {sortedProducts.map(prod => (
                  editingProductId === prod.id ? (
                    <tr key={prod.id} className="border-b border-gray-200 dark:border-gray-700 bg-slate-50 dark:bg-gray-900/60">
                      <td className="px-4 py-2 text-gray-900 dark:text-gray-100">
                        <input
                          type="text"
                          value={editForm.name}
                          onChange={e => setEditForm(prev => ({ ...prev, name: e.target.value }))}
                          className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                          required
                        />
                        <textarea
                          value={editForm.description}
                          onChange={e => setEditForm(prev => ({ ...prev, description: e.target.value }))}
                          className="w-full mt-2 px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                          rows={2}
                          placeholder="Description"
                        />
                      </td>
                      <td className="px-4 py-2 text-gray-900 dark:text-gray-100">
                        <input
                          type="number"
                          step="0.01"
                          value={editForm.price}
                          onChange={e => setEditForm(prev => ({ ...prev, price: e.target.value }))}
                          className="w-32 px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                          required
                          min="0"
                        />
                      </td>
                      <td className="px-4 py-2 text-gray-900 dark:text-gray-100">
                        <label className="flex items-center gap-2 text-sm">
                          <input
                            type="checkbox"
                            checked={editForm.active}
                            onChange={e => setEditForm(prev => ({ ...prev, active: e.target.checked }))}
                          />
                          <span>{editForm.active ? 'Active' : 'Inactive'}</span>
                        </label>
                      </td>
                      <td className="px-4 py-2 text-gray-900 dark:text-gray-100">
                        <div className="flex gap-2">
                          <button
                            type="button"
                            className="px-3 py-2 rounded bg-gray-200 dark:bg-gray-700 text-gray-800 dark:text-gray-100"
                            onClick={resetEditState}
                            disabled={updatingProductId === prod.id}
                          >
                            Cancel
                          </button>
                          <button
                            type="button"
                            className="px-3 py-2 rounded text-white"
                            style={{ background: '#19b4b9' }}
                            onMouseOver={e => (e.currentTarget.style.background = '#153a5b')}
                            onMouseOut={e => (e.currentTarget.style.background = '#19b4b9')}
                            onClick={handleSaveEdit}
                            disabled={updatingProductId === prod.id}
                          >
                            {updatingProductId === prod.id ? 'Saving...' : 'Save'}
                          </button>
                        </div>
                      </td>
                    </tr>
                  ) : (
                    <tr key={prod.id} className="border-b border-gray-200 dark:border-gray-700">
                      <td className="px-4 py-2 text-gray-900 dark:text-gray-100">
                        <div className="font-semibold">{prod.name}</div>
                        {prod.description && <div className="text-sm text-gray-500 dark:text-gray-400">{prod.description}</div>}
                      </td>
                      <td className="px-4 py-2 text-gray-900 dark:text-gray-100">${prod.price.toFixed(2)}</td>
                      <td className="px-4 py-2 text-gray-900 dark:text-gray-100">
                        <span className={`inline-flex items-center rounded-full px-3 py-1 text-xs font-semibold ${prod.active ? 'bg-emerald-100 text-emerald-700' : 'bg-gray-300 text-gray-700'}`}>
                          {prod.active ? 'Active' : 'Inactive'}
                        </span>
                      </td>
                      <td className="px-4 py-2 text-gray-900 dark:text-gray-100">
                        <div className="flex gap-2">
                          <button
                            type="button"
                            className="px-3 py-2 rounded bg-gray-200 dark:bg-gray-700 text-gray-800 dark:text-gray-100"
                            onClick={() => handleStartEdit(prod)}
                            disabled={updatingProductId === prod.id}
                          >
                            Edit
                          </button>
                          <button
                            type="button"
                            className={`px-3 py-2 rounded text-white ${prod.active ? 'bg-red-500 hover:bg-red-600' : 'bg-emerald-500 hover:bg-emerald-600'}`}
                            onClick={() => handleToggleActive(prod)}
                            disabled={updatingProductId === prod.id}
                          >
                            {updatingProductId === prod.id ? 'Updating...' : prod.active ? 'Deactivate' : 'Activate'}
                          </button>
                        </div>
                      </td>
                    </tr>
                  )
                ))}
                {!isLoading && sortedProducts.length === 0 && (
                  <tr>
                    <td colSpan={4} className="px-4 py-2 text-center text-gray-500">No products available.</td>
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



