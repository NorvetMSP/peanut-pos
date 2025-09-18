import React, { useEffect, useMemo, useState } from 'react';
import logoTransparent from '../assets/logo_transparent.png';
import productIcon from '../assets/react.svg';
import ProductCardModern from '../components/ProductCardModern';
import '../components/ProductCardModern.css';
import CartCard from '../components/CartCard';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import { useCart } from '../CartContext';
import { useOrders } from '../OrderContext';

const PRODUCT_SERVICE_URL = (import.meta.env.VITE_PRODUCT_SERVICE_URL ?? 'http://localhost:8081').replace(/\/$/, '');
const FALLBACK_IMAGE = productIcon;
const STORAGE_PREFIX = 'productCache';

type ServiceProduct = {
  id: string;
  tenant_id?: string;
  name: string;
  price: number;
  description: string;
  active: boolean;
  sku?: string | null;
  image_url?: string | null;
};

type CachedProducts = ServiceProduct[];

const getStorage = () => (typeof window !== 'undefined' ? window.localStorage : null);

const SalesPage: React.FC = () => {
  const { logout, isLoggedIn, currentUser, token } = useAuth();
  const { cart, addItem, removeItem, incrementItemQuantity, decrementItemQuantity, totalAmount } = useCart();
  const { queuedOrders, isOnline, isSyncing } = useOrders();

  const [products, setProducts] = useState<ServiceProduct[]>([]);
  const [query, setQuery] = useState('');
  const [isLoadingProducts, setIsLoadingProducts] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isOfflineResult, setIsOfflineResult] = useState(false);

  const navigate = useNavigate();

  useEffect(() => {
    if (!isLoggedIn) navigate('/login');
  }, [isLoggedIn, navigate]);

  useEffect(() => {
    const tenantId = currentUser?.tenant_id;
    if (!tenantId) {
      setProducts([]);
      return;
    }

    let isMounted = true;
    const storage = getStorage();
    const cacheKey = `${STORAGE_PREFIX}:${tenantId}`;

    const normalizeProducts = (raw: unknown): ServiceProduct[] => {
      if (!Array.isArray(raw)) return [];
      const normalized: ServiceProduct[] = [];
      for (const item of raw) {
        if (!item || typeof item !== 'object') continue;
        const candidate = item as Record<string, unknown>;
        const id = candidate.id;
        const name = candidate.name;
        const priceValue = typeof candidate.price === 'number' ? candidate.price : Number(candidate.price);
        if (!id || typeof name !== 'string' || Number.isNaN(priceValue)) {
          continue;
        }
        const description = typeof candidate.description === 'string' ? candidate.description : '';
        normalized.push({
          id: String(id),
          tenant_id: candidate.tenant_id ? String(candidate.tenant_id) : undefined,
          name,
          price: priceValue,
          description,
          active: typeof candidate.active === 'boolean' ? candidate.active : true,
          sku: typeof candidate.sku === 'string' ? candidate.sku : null,
          image_url: typeof candidate.image_url === 'string' ? candidate.image_url : null,
        });
      }
      return normalized;
    };

    const loadFromCache = (): boolean => {
      if (!storage) return false;
      const cached = storage.getItem(cacheKey);
      if (!cached) return false;
      try {
        const parsed = JSON.parse(cached) as CachedProducts;
        if (!isMounted) return true;
        const active = parsed.filter((p) => p.active);
        setProducts(active);
        return active.length > 0;
      } catch (err) {
        console.warn('Unable to parse cached products', err);
        return false;
      }
    };

    const loadProducts = async () => {
      setIsLoadingProducts(true);
      setError(null);
      setIsOfflineResult(false);

      const headers: HeadersInit = { 'X-Tenant-ID': tenantId };
      if (token) headers['Authorization'] = `Bearer ${token}`;

      try {
        const response = await fetch(`${PRODUCT_SERVICE_URL}/products`, {
          headers,
        });

        if (!response.ok) {
          throw new Error(`Failed to fetch products (${response.status})`);
        }

        const data = await response.json();
        const normalized = normalizeProducts(data);
        if (storage) {
          storage.setItem(cacheKey, JSON.stringify(normalized));
        }
        if (isMounted) {
          const active = normalized.filter((p) => p.active);
          setProducts(active);
        }
      } catch (err) {
        console.warn('Unable to load products', err);
        const hadCached = loadFromCache();
        if (isMounted) {
          if (!hadCached) {
            setError('Product catalog unavailable offline.');
          } else {
            setIsOfflineResult(true);
          }
        }
      } finally {
        if (isMounted) {
          setIsLoadingProducts(false);
        }
      }
    };

    loadProducts();

    return () => {
      isMounted = false;
    };
  }, [currentUser?.tenant_id, token]);

  const filteredProducts = useMemo(() => {
    const trimmed = query.trim().toLowerCase();
    if (!trimmed) return products;
    return products.filter((p) => {
      const nameMatch = p.name.toLowerCase().includes(trimmed);
      const skuMatch = p.sku ? p.sku.toLowerCase().includes(trimmed) : false;
      return nameMatch || skuMatch;
    });
  }, [products, query]);

  const incrementQty = (item: { id: string }) => {
    incrementItemQuantity(item.id);
  };
  const decrementQty = (item: { id: string; quantity: number }) => {
    if (item.quantity > 1) {
      decrementItemQuantity(item.id);
    }
  };

  return (
    <div className="min-h-screen bg-gradient-to-br from-gray-100 to-gray-300 dark:from-gray-900 dark:to-gray-800 flex flex-col">
      <header className="flex items-center justify-between px-6 py-4 bg-white dark:bg-gray-900 shadow-md">
        <div className="flex items-center gap-3">
          <img src={logoTransparent} alt="NovaPOS Logo" className="h-10 w-auto" />
          <span className="text-2xl font-bold text-primary dark:text-white tracking-tight">NovaPOS</span>
        </div>
        <div className="flex items-center gap-3">
          <button
            className="px-4 py-2 rounded border border-cyan-500 text-cyan-700 hover:bg-cyan-500 hover:text-white transition-colors"
            onClick={() => navigate('/history')}
          >
            Orders{queuedOrders.length > 0 ? ` (${queuedOrders.length})` : ''}
          </button>
          <button className="bg-red-500 text-white px-4 py-2 rounded hover:bg-red-600" onClick={() => { logout(); navigate('/login'); }}>Logout</button>
        </div>
      </header>

      {!isOnline && (
        <div className="w-full bg-amber-200 text-amber-900 px-6 py-3 text-sm text-center">
          Offline mode - {queuedOrders.length} order{queuedOrders.length === 1 ? '' : 's'} queued. Sales will sync automatically once reconnected.
        </div>
      )}
      {isOnline && queuedOrders.length > 0 && (
        <div className="w-full bg-sky-200 text-sky-900 px-6 py-3 text-sm text-center">
          {isSyncing ? 'Synchronizing queued orders...' : `${queuedOrders.length} queued order${queuedOrders.length === 1 ? '' : 's'} awaiting sync.`}
        </div>
      )}

      <div className="flex-1 flex flex-col w-full items-center">
        <main className="flex flex-col gap-6 px-4 py-8 w-full max-w-5xl mx-auto">
          <section className="w-full">
            <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 mb-4">
              <h2 className="text-xl font-semibold text-gray-800 dark:text-gray-100">Products</h2>
              <input
                type="search"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Search products by name or SKU"
                className="w-full sm:w-80 px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-indigo-500"
                aria-label="Search products"
              />
            </div>
            {isLoadingProducts && (
              <div className="text-sm text-gray-500 mb-2">Loading products...</div>
            )}
            {isOfflineResult && !isLoadingProducts && (
              <div className="text-sm text-amber-600 mb-2">Offline mode: showing last synced catalog.</div>
            )}
            {error && !isLoadingProducts && (
              <div className="text-sm text-red-600 mb-2">{error}</div>
            )}
            <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-6">
              {filteredProducts.map(product => (
                <ProductCardModern
                  key={product.id}
                  product={{
                    id: product.id,
                    name: product.name,
                    description: product.description,
                    price: product.price,
                    sku: product.sku ?? undefined,
                    image: product.image_url ?? FALLBACK_IMAGE,
                    onAdd: () => addItem({
                      id: product.id,
                      name: product.name,
                      price: product.price,
                      sku: product.sku ?? undefined,
                    }),
                    onWishlist: () => alert(`Added ${product.name} to wishlist!`),
                  }}
                />
              ))}
              {!isLoadingProducts && filteredProducts.length === 0 && (
                <div className="col-span-full text-center text-gray-500">
                  No products found. Try adjusting your search.
                </div>
              )}
            </div>
          </section>

          <CartCard
            items={cart.map(item => ({
              ...item,
              onRemove: () => removeItem(item.id),
              onAdd: () => incrementQty(item),
              onSubtract: () => decrementQty(item),
            }))}
            total={totalAmount}
            onCheckout={() => navigate('/checkout')}
          />
        </main>
      </div>
    </div>
  );
};

export default SalesPage;
