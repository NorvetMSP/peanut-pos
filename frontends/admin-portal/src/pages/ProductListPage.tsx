import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import { resolveServiceUrl } from '../utils/env';
import './AdminSectionModern.css';

const PRODUCT_SERVICE_URL = resolveServiceUrl('VITE_PRODUCT_SERVICE_URL', 'http://localhost:8081');
const DEFAULT_IMAGE_PLACEHOLDER = 'https://images.fineartamerica.com/images/artworkimages/mediumlarge/1/super-nova-rina-kaff.jpg"';

type ServiceProduct = {
  id: string;
  name: string;
  price: number;
  description: string;
  active: boolean;
  image: string;
};

type ProductFormState = {
  name: string;
  price: string;
  description: string;
  image: string;
};

type EditFormState = ProductFormState & { active: boolean };

type ProductJson = Record<string, unknown>;

type ProductAuditEntry = {
  id: string;
  action: string;
  created_at: string;
  actor_id?: string | null;
  actor_name?: string | null;
  actor_email?: string | null;
  changes: unknown;
};

const normalizeProduct = (input: unknown): ServiceProduct | null => {
  if (!input || typeof input !== 'object') return null;
  const candidate = input as ProductJson;
  const id = candidate.id;
  const name = candidate.name;
  const priceRaw = candidate.price;
  if (typeof id !== 'string' || typeof name !== 'string') return null;
  const price = typeof priceRaw === 'number' ? priceRaw : Number(priceRaw);
  if (!Number.isFinite(price)) return null;
  const rawImage =
    typeof candidate.image === 'string'
      ? candidate.image
      : typeof candidate.image_url === 'string'
      ? candidate.image_url
      : '';
  const image = rawImage.trim();
  return {
    id,
    name,
    price,
    description: typeof candidate.description === 'string' ? candidate.description : '',
    active: typeof candidate.active === 'boolean' ? candidate.active : true,
    image,
  };
};

const ProductListPage: React.FC = () => {
  const { isLoggedIn, currentUser, token } = useAuth();
  const navigate = useNavigate();

  const [products, setProducts] = useState<ServiceProduct[]>([]);
  const [newProduct, setNewProduct] = useState<ProductFormState>({ name: '', price: '', description: '', image: '' });
  const [editingProductId, setEditingProductId] = useState<string | null>(null);
  const [editForm, setEditForm] = useState<EditFormState>({ name: '', price: '', description: '', image: '', active: true });
  const [isLoading, setIsLoading] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [updatingProductId, setUpdatingProductId] = useState<string | null>(null);
  const [deletingProductId, setDeletingProductId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  const [auditLogByProduct, setAuditLogByProduct] = useState<Record<string, ProductAuditEntry[]>>({});
  const [auditLoadingByProduct, setAuditLoadingByProduct] = useState<Record<string, boolean>>({});
  const [auditErrorByProduct, setAuditErrorByProduct] = useState<Record<string, string | null>>({});

  const [historyProductId, setHistoryProductId] = useState<string | null>(null);

  useEffect(() => {
    if (!isLoggedIn) void navigate('/login', { replace: true });
  }, [isLoggedIn, navigate]);

  const tenantId = currentUser?.tenant_id ? String(currentUser.tenant_id) : null;

  const buildHeaders = useCallback((): Record<string, string> => {
    const headers: Record<string, string> = {};
    if (tenantId) headers['X-Tenant-ID'] = tenantId;
    const userId = currentUser && typeof currentUser.id === 'string' ? currentUser.id.trim() : '';
    if (userId) headers['X-User-ID'] = userId;
    const userEmail = currentUser && typeof currentUser.email === 'string' ? currentUser.email.trim() : '';
    if (userEmail) headers['X-User-Email'] = userEmail;
    const userName = currentUser && typeof currentUser.name === 'string' ? currentUser.name.trim() : '';
    if (userName) headers['X-User-Name'] = userName;
    if (token) headers['Authorization'] = `Bearer ${token}`;
    return headers;
  }, [currentUser, tenantId, token]);

  const ensureTenantContext = useCallback((): boolean => {
    if (!tenantId) {
      setError('Tenant context is unavailable. Please log out and back in.');
      setProducts([]);
      return false;
    }
    return true;
  }, [tenantId]);

  const fetchProducts = useCallback(async (): Promise<void> => {
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
      const data = (await response.json()) as unknown;
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

  // Helper to normalize audit entries from API response
  function normalizeAuditEntries(input: unknown): ProductAuditEntry[] {
    const hasEntriesArray = (value: unknown): value is { entries: unknown[] } => {
      if (typeof value !== 'object' || value === null) return false;
      if (!('entries' in value)) return false;
      const { entries } = value as { entries: unknown };
      return Array.isArray(entries);
    };

    if (!input) return [];
    if (Array.isArray(input)) {
      return input
        .map(normalizeAuditEntry)
        .filter((entry): entry is ProductAuditEntry => entry !== null);
    }
    if (hasEntriesArray(input)) {
      return input.entries
        .map(normalizeAuditEntry)
        .filter((entry): entry is ProductAuditEntry => entry !== null);
    }
    return [];
  }

  // Helper to normalize a single audit entry
  function normalizeAuditEntry(entry: unknown): ProductAuditEntry | null {
    if (typeof entry !== 'object' || entry === null) return null;

    const candidate = entry as Record<string, unknown>;
    const id = candidate.id;
    const action = candidate.action;
    const createdAt = candidate.created_at;

    if (typeof id !== 'string' || typeof action !== 'string' || typeof createdAt !== 'string') {
      return null;
    }

    const actorId = candidate.actor_id;
    const actorName = candidate.actor_name;
    const actorEmail = candidate.actor_email;
    const changes = candidate.changes;

    return {
      id,
      action,
      created_at: createdAt,
      actor_id: typeof actorId === 'string' ? actorId : null,
      actor_name: typeof actorName === 'string' ? actorName : null,
      actor_email: typeof actorEmail === 'string' ? actorEmail : null,
      changes: changes === undefined || changes === null ? {} : changes,
    };
  }

  const fetchProductAudit = useCallback(
    async (productId: string, force = false): Promise<void> => {
      if (!ensureTenantContext()) return;
      if (!force && auditLogByProduct[productId]) return;
      setAuditLoadingByProduct(prev => ({ ...prev, [productId]: true }));
      setAuditErrorByProduct(prev => ({ ...prev, [productId]: null }));
      try {
        const response = await fetch(`${PRODUCT_SERVICE_URL}/products/${productId}/audit?limit=10`, {
          headers: buildHeaders(),
        });
        if (!response.ok) {
          throw new Error(`Failed to fetch product audit (${response.status})`);
        }
        const payload = (await response.json()) as unknown;
        const entries = normalizeAuditEntries(payload);
        setAuditLogByProduct(prev => ({ ...prev, [productId]: entries }));
      } catch (err) {
        console.error('Unable to load product audit', err);
        setAuditErrorByProduct(prev => ({ ...prev, [productId]: 'Unable to load product history.' }));
      } finally {
        setAuditLoadingByProduct(prev => ({ ...prev, [productId]: false }));
      }
    },
    [auditLogByProduct, buildHeaders, ensureTenantContext],
  );

  const handleToggleHistory = (productId: string): void => {
    const nextId = historyProductId === productId ? null : productId;
    setHistoryProductId(nextId);
    if (nextId) {
      void fetchProductAudit(productId);
    }
  };

  useEffect(() => {
    void fetchProducts();
  }, [fetchProducts]);

  const resetEditState = () => {
    setEditingProductId(null);
    setEditForm({ name: '', price: '', description: '', image: '', active: true });
  };

  const handleAddProduct = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
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
      const imageValue = newProduct.image.trim();
      const requestBody: Record<string, unknown> = {
        name: trimmedName,
        price: priceValue,
        description: newProduct.description.trim(),
      };
      if (imageValue.length > 0) {
        requestBody.image = imageValue;
      }
      const response = await fetch(`${PRODUCT_SERVICE_URL}/products`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...buildHeaders(),
        },
        body: JSON.stringify(requestBody),
      });
      if (!response.ok) {
        throw new Error(`Failed to add product (${response.status})`);
      }
      const payload = (await response.json()) as unknown;
      const created = normalizeProduct(payload);
      if (created) {
        setProducts(prev => [...prev, created]);
      }
      setNewProduct({ name: '', price: '', description: '', image: '' });
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
      image: product.image ?? '',
      active: product.active,
    });
    void fetchProductAudit(product.id);
  };

  const handleSaveEdit = async (): Promise<void> => {
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
      const imageValue = editForm.image.trim();
      const requestBody: Record<string, unknown> = {
        name: trimmedName,
        price: priceValue,
        description: editForm.description.trim(),
        active: editForm.active,
      };
      if (imageValue.length > 0) {
        requestBody.image = imageValue;
      } else {
        requestBody.image = '';
      }
      const response = await fetch(`${PRODUCT_SERVICE_URL}/products/${editingProductId}`, {
        method: 'PUT',
        headers: {
          'Content-Type': 'application/json',
          ...buildHeaders(),
        },
        body: JSON.stringify(requestBody),
      });
      if (!response.ok) {
        throw new Error(`Failed to update product (${response.status})`);
      }
      const payload = (await response.json()) as unknown;
      const updated = normalizeProduct(payload);
      if (updated) {
        setProducts(prev => prev.map(prod => (prod.id === updated.id ? updated : prod)));
        void fetchProductAudit(updated.id, true);
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

  const handleToggleActive = async (product: ServiceProduct): Promise<void> => {
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
          image: product.image ?? '',
        }),
      });
      if (!response.ok) {
        throw new Error(`Failed to update product (${response.status})`);
      }
      const payload = (await response.json()) as unknown;
      const updated = normalizeProduct(payload);
      if (updated) {
        setProducts(prev => prev.map(prod => (prod.id === updated.id ? updated : prod)));
        void fetchProductAudit(product.id, true);
      }
      setSuccessMessage(`Product ${product.active ? 'deactivated' : 'activated'} successfully.`);
    } catch (err) {
      console.error('Unable to toggle product status', err);
      setError('Unable to update product status. Please try again.');
    } finally {
      setUpdatingProductId(null);
    }
  };

  const handleDeleteProduct = async (product: ServiceProduct): Promise<void> => {
    if (!ensureTenantContext()) return;

    const confirmed = typeof window === 'undefined'
      ? true
      : window.confirm(`Delete "${product.name}"? This action cannot be undone.`);
    if (!confirmed) return;

    setError(null);
    setSuccessMessage(null);
    setDeletingProductId(product.id);
    try {
      const headers = buildHeaders();
      const response = await fetch(`${PRODUCT_SERVICE_URL}/products/${product.id}`, {
        method: 'DELETE',
        headers,
      });

      if (response.status === 204) {
        setProducts(prev => prev.filter(item => item.id !== product.id));
        setAuditLogByProduct(prev => {
          const { [product.id]: _unused, ...rest } = prev;
          return rest;
        });
        setAuditLoadingByProduct(prev => {
          const { [product.id]: _unused, ...rest } = prev;
          return rest;
        });
        setAuditErrorByProduct(prev => {
          const { [product.id]: _unused, ...rest } = prev;
          return rest;
        });
        if (editingProductId === product.id) {
          resetEditState();
        }
        setSuccessMessage('Product deleted successfully.');
      } else if (response.status === 404) {
        setError('Product not found or already removed.');
      } else {
        const detail = await response.text();
        throw new Error(detail || 'Delete failed');
      }
    } catch (err) {
      console.error('Unable to delete product', err);
      setError('Unable to delete product. Please try again.');
    } finally {
      setDeletingProductId(null);
    }
  };
  const handleRefresh = () => {
    setSuccessMessage(null);
    setError(null);
    void fetchProducts();
  };

  const newProductPreviewSrc = newProduct.image.trim().length > 0 ? newProduct.image.trim() : DEFAULT_IMAGE_PLACEHOLDER;
  const sortedProducts = useMemo(() => {
    return [...products].sort((a, b) => a.name.localeCompare(b.name));
  }, [products]);

  return (
    <div className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col">
      <div className="admin-section-modern">
        <div className="admin-section-header">
          <h2>Products</h2>
          <p>Manage your catalog for this tenant.</p>
        </div>

        <div className="admin-section-content">
          {error && (
            <div className="rounded bg-red-100 text-red-800 px-4 py-3 mb-4">{error}</div>
          )}
          {successMessage && (
            <div className="rounded bg-green-100 text-green-800 px-4 py-3 mb-4">{successMessage}</div>
          )}

          <section className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Add New Product</h3>
            <form className="mt-4 grid gap-4 md:grid-cols-2" onSubmit={event => { void handleAddProduct(event); }}>
              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">Product Name</label>
                <input
                  type="text"
                  value={newProduct.name}
                  onChange={event => setNewProduct(prev => ({ ...prev, name: event.target.value }))}
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                  placeholder="Deluxe Latte"
                  required
                />
              </div>
              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">Price</label>
                <input
                  type="number"
                  value={newProduct.price}
                  min="0"
                  step="0.01"
                  onChange={event => setNewProduct(prev => ({ ...prev, price: event.target.value }))}
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                  placeholder="9.99"
                  required
                />
              </div>
              <div className="md:col-span-2 flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">Description</label>
                <textarea
                  value={newProduct.description}
                  onChange={event => setNewProduct(prev => ({ ...prev, description: event.target.value }))}
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                  rows={3}
                  placeholder="Describe the product"
                />
              </div>
              <div className="md:col-span-2 flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">Image URL (optional)</label>
                <input
                  type="url"
                  value={newProduct.image}
                  onChange={event => setNewProduct(prev => ({ ...prev, image: event.target.value }))}
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                  placeholder="https://cdn.example.com/product.jpg"
                />
                <span className="mt-1 text-xs text-gray-500 dark:text-gray-400">Leave blank to use the default placeholder image.</span>
              </div>
              <div className="md:col-span-2 flex items-center gap-4">
                <img
                  src={newProductPreviewSrc}
                  alt="Product preview"
                  className="h-16 w-16 rounded-md border border-gray-200 object-cover"
                  onError={event => {
                    event.currentTarget.src = DEFAULT_IMAGE_PLACEHOLDER;
                  }}
                />
                <span className="text-sm text-gray-600 dark:text-gray-400">Preview updates automatically when you enter a URL.</span>
              </div>
              <div className="md:col-span-2 flex justify-end gap-2">
                <button
                  type="button"
                  className="px-4 py-2 rounded bg-gray-200 text-gray-800 dark:bg-gray-700 dark:text-gray-200"
                  onClick={() => setNewProduct({ name: '', price: '', description: '', image: '' })}
                  disabled={isSubmitting}
                >
                  Clear
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 rounded text-white"
                  style={{ background: '#19b4b9' }}
                  onMouseOver={event => (event.currentTarget.style.background = '#153a5b')}
                  onMouseOut={event => (event.currentTarget.style.background = '#19b4b9')}
                  disabled={isSubmitting}
                >
                  {isSubmitting ? 'Adding...' : 'Add Product'}
                </button>
              </div>
            </form>
          </section>

          <section className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow mt-6">
            <div className="flex items-center justify-between">
              <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Current Products</h3>
              <button className="admin-section-btn" type="button" onClick={handleRefresh} disabled={isLoading}>
                {isLoading ? 'Refreshing...' : 'Refresh List'}
              </button>
            </div>

            <div className="mt-4 overflow-x-auto">
              <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                <thead className="bg-gray-50 dark:bg-gray-700">
                  <tr>
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">Product</th>
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">Price</th>
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">Status</th>
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">Actions</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                  {sortedProducts.map(prod => {
                    const isEditing = editingProductId === prod.id;
                    const auditEntries = auditLogByProduct[prod.id] ?? [];
                    const auditLoading = auditLoadingByProduct[prod.id];
                    const auditError = auditErrorByProduct[prod.id];
                    const showHistory = isEditing || historyProductId === prod.id;

                    if (isEditing) {
                      return (
                        <React.Fragment key={prod.id}>
                          <tr className="bg-gray-50 dark:bg-gray-900">
                            <td className="px-4 py-2 text-gray-900 dark:text-gray-100">
                              <div className="flex items-start gap-3">
                                <img
                                  src={editForm.image.trim().length > 0 ? editForm.image.trim() : DEFAULT_IMAGE_PLACEHOLDER}
                                  alt={(editForm.name || 'Product') + ' preview'}
                                  className="h-16 w-16 rounded-md border border-gray-200 object-cover"
                                  onError={event => {
                                    event.currentTarget.src = DEFAULT_IMAGE_PLACEHOLDER;
                                  }}
                                />
                                <div className="flex-1 space-y-2">
                                  <input
                                    value={editForm.name}
                                    onChange={event => setEditForm(prev => ({ ...prev, name: event.target.value }))}
                                    className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                                    required
                                  />
                                  <textarea
                                    value={editForm.description}
                                    onChange={event => setEditForm(prev => ({ ...prev, description: event.target.value }))}
                                    className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                                    rows={2}
                                    placeholder="Description"
                                  />
                                  <input
                                    type="url"
                                    value={editForm.image}
                                    onChange={event => setEditForm(prev => ({ ...prev, image: event.target.value }))}
                                    className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                                    placeholder="https://cdn.example.com/product.jpg"
                                  />
                                  <p className="text-xs text-gray-500 dark:text-gray-400">Clear the field to revert to the default placeholder image.</p>
                                </div>
                              </div>
                            </td>
                            <td className="px-4 py-2 text-gray-900 dark:text-gray-100">
                              <input
                                type="number"
                                step="0.01"
                                value={editForm.price}
                                onChange={event => setEditForm(prev => ({ ...prev, price: event.target.value }))}
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
                                  onChange={event => setEditForm(prev => ({ ...prev, active: event.target.checked }))}
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
                                  disabled={updatingProductId === prod.id || deletingProductId === prod.id}
                                >
                                  Cancel
                                </button>
                                <button
                                  type="button"
                                  className="px-3 py-2 rounded text-white"
                                  style={{ background: '#19b4b9' }}
                                  onMouseOver={event => (event.currentTarget.style.background = '#153a5b')}
                                  onMouseOut={event => (event.currentTarget.style.background = '#19b4b9')}
                                  onClick={() => void handleSaveEdit()}
                                  disabled={updatingProductId === prod.id || deletingProductId === prod.id}
                                >
                                  {updatingProductId === prod.id ? 'Saving...' : 'Save'}
                                </button>
                              </div>
                            </td>
                          </tr>
                          <tr className="bg-gray-50 dark:bg-gray-900">
                            <td colSpan={4} className="px-4 pb-4 text-gray-900 dark:text-gray-100">
                              <div className="mt-4 rounded border border-gray-200 dark:border-gray-700 bg-gray-100 dark:bg-gray-800 p-4">
                                <h4 className="text-sm font-semibold mb-2">Recent Changes</h4>
                                {auditLoading ? (
                                  <p className="text-sm text-gray-500 dark:text-gray-400">Loading history...</p>
                                ) : auditError ? (
                                  <p className="text-sm text-red-600">{auditError}</p>
                                ) : auditEntries.length === 0 ? (
                                  <p className="text-sm text-gray-500 dark:text-gray-400">No audit history yet.</p>
                                ) : (
                                  <ul className="space-y-3 text-sm">
                                    {auditEntries.map(entry => {
                                      const actorLabel = entry.actor_name || entry.actor_email || 'Unknown user';
                                      return (
                                        <li key={entry.id} className="rounded bg-white/80 dark:bg-gray-900/60 border border-gray-200 dark:border-gray-700 p-3">
                                          <div className="flex justify-between items-start">
                                            <span className="font-medium">{entry.action.toUpperCase()}</span>
                                            <span className="text-xs text-gray-500 dark:text-gray-400">{new Date(entry.created_at).toLocaleString()}</span>
                                          </div>
                                          <div className="text-xs text-gray-600 dark:text-gray-400 mt-1">{actorLabel}</div>
                                          <pre className="mt-2 max-h-32 overflow-auto rounded bg-gray-900/80 text-gray-100 text-xs p-2">{JSON.stringify(entry.changes, null, 2)}</pre>
                                        </li>
                                      );
                                    })}
                                  </ul>
                                )}
                              </div>
                            </td>
                          </tr>
                        </React.Fragment>
                      );
                    }

                    const isHistoryVisible = historyProductId === prod.id;

                    return (
                      <React.Fragment key={prod.id}>
                        <tr className="border-b border-gray-200 dark:border-gray-700">
                          <td className="px-4 py-2 text-gray-900 dark:text-gray-100">
                            <div className="flex items-start gap-3">
                              <img
                                src={prod.image.trim().length > 0 ? prod.image.trim() : DEFAULT_IMAGE_PLACEHOLDER}
                                alt={prod.name + ' preview'}
                                className="h-12 w-12 rounded-md border border-gray-200 object-cover"
                                onError={event => {
                                  event.currentTarget.src = DEFAULT_IMAGE_PLACEHOLDER;
                                }}
                              />
                              <div>
                                <div className="font-semibold">{prod.name}</div>
                                {prod.description && <div className="text-sm text-gray-500 dark:text-gray-400">{prod.description}</div>}
                              </div>
                            </div>
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
                                disabled={updatingProductId === prod.id || deletingProductId === prod.id}
                              >
                                Edit
                              </button>
                              <button
                                type="button"
                                className="px-3 py-2 rounded bg-indigo-600 text-white hover:bg-indigo-700"
                                onClick={() => handleToggleHistory(prod.id)}
                                disabled={updatingProductId === prod.id || deletingProductId === prod.id}
                              >
                                {isHistoryVisible ? 'Hide History' : 'History'}
                              </button>
                              <button
                                type="button"
                                className={`px-3 py-2 rounded text-white ${prod.active ? 'bg-red-500 hover:bg-red-600' : 'bg-emerald-500 hover:bg-emerald-600'}`}
                                onClick={() => void handleToggleActive(prod)}
                                disabled={updatingProductId === prod.id || deletingProductId === prod.id}
                              >
                                {updatingProductId === prod.id ? 'Updating...' : prod.active ? 'Deactivate' : 'Activate'}
                              </button>
                              <button
                                type="button"
                                className="px-3 py-2 rounded bg-red-600 text-white hover:bg-red-700"
                                onClick={() => void handleDeleteProduct(prod)}
                                disabled={deletingProductId === prod.id || updatingProductId === prod.id}
                              >
                                {deletingProductId === prod.id ? 'Deleting...' : 'Delete'}
                              </button>
                            </div>
                          </td>
                        </tr>
                        {isHistoryVisible && (
                          <tr className="border-b border-gray-200 dark:border-gray-700">
                            <td colSpan={4} className="px-4 pb-4 text-gray-900 dark:text-gray-100">
                              <div className="mt-4 rounded border border-gray-200 dark:border-gray-700 bg-gray-100 dark:bg-gray-800 p-4">
                                <h4 className="text-sm font-semibold mb-2">Recent Changes</h4>
                                {auditLoading ? (
                                  <p className="text-sm text-gray-500 dark:text-gray-400">Loading history...</p>
                                ) : auditError ? (
                                  <p className="text-sm text-red-600">{auditError}</p>
                                ) : auditEntries.length === 0 ? (
                                  <p className="text-sm text-gray-500 dark:text-gray-400">No audit history yet.</p>
                                ) : (
                                  <ul className="space-y-3 text-sm">
                                    {auditEntries.map(entry => {
                                      const actorLabel = entry.actor_name || entry.actor_email || 'Unknown user';
                                      return (
                                        <li key={entry.id} className="rounded bg-white/80 dark:bg-gray-900/60 border border-gray-200 dark:border-gray-700 p-3">
                                          <div className="flex justify-between items-start">
                                            <span className="font-medium">{entry.action.toUpperCase()}</span>
                                            <span className="text-xs text-gray-500 dark:text-gray-400">{new Date(entry.created_at).toLocaleString()}</span>
                                          </div>
                                          <div className="text-xs text-gray-600 dark:text-gray-400 mt-1">{actorLabel}</div>
                                          <pre className="mt-2 max-h-32 overflow-auto rounded bg-gray-900/80 text-gray-100 text-xs p-2">{JSON.stringify(entry.changes, null, 2)}</pre>
                                        </li>
                                      );
                                    })}
                                  </ul>
                                )}
                              </div>
                            </td>
                          </tr>
                        )}
                      </React.Fragment>
                    );
                  })}
                  {!isLoading && sortedProducts.length === 0 && (
                    <tr>
                      <td colSpan={4} className="px-4 py-2 text-center text-gray-500">No products available.</td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </section>
        </div>
        <div style={{ textAlign: 'right', marginTop: '2rem' }}>
          <button className="admin-section-btn" onClick={() => void navigate('/home')} type="button">
            Back to Admin Home
          </button>
        </div>
      </div>
    </div>
  );
};

export default ProductListPage;
