import React, { useEffect, useMemo, useRef, useState } from 'react';
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

const parsePositiveNumber = (raw: unknown, fallback: number): number => {
  if (typeof raw === 'number' && Number.isFinite(raw) && raw > 0) {
    return raw;
  }
  if (typeof raw === 'string' && raw.trim().length > 0) {
    const parsed = Number(raw);
    if (Number.isFinite(parsed) && parsed > 0) {
      return parsed;
    }
  }
  return fallback;
};

const CATALOG_REFRESH_INTERVAL_MS = parsePositiveNumber(import.meta.env.VITE_CATALOG_REFRESH_INTERVAL_MS, 180000);
const CATALOG_PERF_BUDGET_MS = parsePositiveNumber(import.meta.env.VITE_CATALOG_PERF_BUDGET_MS, 5000);
const SALES_TAX_RATE = parsePositiveNumber(import.meta.env.VITE_SALES_TAX_RATE, 0.07);

const formatRelativeTime = (timestamp: number): string => {
  const diff = Date.now() - timestamp;
  if (diff < 0) return 'just now';
  const seconds = Math.round(diff / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return `${days}d ago`;
};

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

type CatalogCacheEnvelope = {
  version: 1;
  updatedAt: number;
  products: CachedProducts;
};

type CatalogIndex = Record<string, { active: boolean; price: number }>;

const getStorage = () => (typeof window !== 'undefined' ? window.localStorage : null);

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

const deserializeCache = (raw: string): { products: ServiceProduct[]; updatedAt: number | null } => {
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (parsed && typeof parsed === 'object' && Array.isArray((parsed as { products?: unknown }).products)) {
      const envelope = parsed as { products: unknown; updatedAt?: unknown };
      const normalized = normalizeProducts(envelope.products);
      const updatedAt = typeof envelope.updatedAt === 'number' ? envelope.updatedAt : null;
      return { products: normalized, updatedAt };
    }
    return { products: normalizeProducts(parsed), updatedAt: null };
  } catch (err) {
    console.warn('Unable to parse cached products', err);
    return { products: [], updatedAt: null };
  }
};

const persistCatalog = (storage: Storage | null, cacheKey: string, products: ServiceProduct[]): number | null => {
  if (!storage) return null;
  const envelope: CatalogCacheEnvelope = {
    version: 1,
    updatedAt: Date.now(),
    products,
  };
  storage.setItem(cacheKey, JSON.stringify(envelope));
  return envelope.updatedAt;
};

const SalesPage: React.FC = () => {
  const { logout, isLoggedIn, currentUser, token } = useAuth();
  const { cart, addItem, removeItem, incrementItemQuantity, decrementItemQuantity, clearCart, totalAmount } = useCart();
  const { queuedOrders, isOnline, isSyncing } = useOrders();

  const [products, setProducts] = useState<ServiceProduct[]>([]);
  const [catalogIndex, setCatalogIndex] = useState<CatalogIndex>({});
  const [query, setQuery] = useState('');
  const [isLoadingProducts, setIsLoadingProducts] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isOfflineResult, setIsOfflineResult] = useState(false);
  const [lastSyncTime, setLastSyncTime] = useState<number | null>(null);
  const [lastFetchDuration, setLastFetchDuration] = useState<number | null>(null);

  const navigate = useNavigate();

  useEffect(() => {
    if (!isLoggedIn) navigate('/login');
  }, [isLoggedIn, navigate]);

  useEffect(() => {
    const tenantId = currentUser?.tenant_id;
    if (!tenantId) {
      setProducts([]);
      setLastSyncTime(null);
      setLastFetchDuration(null);
      setCatalogIndex({});
      return;
    }

    let isMounted = true;
    const storage = getStorage();
    const cacheKey = `${STORAGE_PREFIX}:${tenantId}`;
    let refreshTimer: ReturnType<typeof window.setInterval> | null = null;

    const cacheToIndex = (list: ServiceProduct[]): CatalogIndex => Object.fromEntries(
      list.map(item => [item.id, { active: item.active, price: item.price }])
    );

    const loadCache = (reason: 'warmup' | 'fallback' = 'warmup'): boolean => {
      if (!storage) return false;
      const cached = storage.getItem(cacheKey);
      if (!cached) return false;
      const { products: cachedProducts, updatedAt } = deserializeCache(cached);
      if (!cachedProducts.length) return false;
      if (isMounted) {
        const active = cachedProducts.filter(p => p.active);
        setProducts(active);
        setCatalogIndex(cacheToIndex(cachedProducts));
        if (updatedAt) setLastSyncTime(updatedAt);
        if (reason === 'fallback') {
          setIsOfflineResult(true);
          setError(null);
          setLastFetchDuration(null);
        }
      }
      return true;
    };

    const loadProducts = async (options?: { background?: boolean; eagerCache?: boolean }) => {
      const isBackground = Boolean(options?.background);
      let hadCache = false;
      if (options?.eagerCache) {
        hadCache = loadCache('warmup');
      }

      if (isBackground) {
        setIsRefreshing(true);
      } else {
        setError(null);
        setIsOfflineResult(false);
        setIsLoadingProducts(!hadCache);
      }

      const headers: HeadersInit = { 'X-Tenant-ID': String(tenantId) };
      if (token) headers['Authorization'] = `Bearer ${token}`;
      const timerStart = typeof performance !== 'undefined' && typeof performance.now === 'function' ? performance.now() : Date.now();

      try {
        const response = await fetch(`${PRODUCT_SERVICE_URL}/products`, { headers });
        if (!response.ok) {
          throw new Error(`Failed to fetch products (${response.status})`);
        }

        const data = await response.json();
        const normalized = normalizeProducts(data);
        const updatedAt = persistCatalog(storage, cacheKey, normalized) ?? Date.now();
        const index = cacheToIndex(normalized);

        if (isMounted) {
          const active = normalized.filter(p => p.active);
          setProducts(active);
          setCatalogIndex(index);
          setLastSyncTime(updatedAt);
          setIsOfflineResult(false);
          const duration = (typeof performance !== 'undefined' && typeof performance.now === 'function')
            ? performance.now() - timerStart
            : Date.now() - timerStart;
          setLastFetchDuration(duration);
          if (duration > CATALOG_PERF_BUDGET_MS) {
            console.warn(`Catalog sync exceeded budget: ${duration.toFixed(0)}ms`);
          }
        }
      } catch (err) {
        console.warn('Unable to load products', err);
        const hadFallback = loadCache('fallback');
        if (isMounted && !hadFallback) {
          setError('Product catalog unavailable offline.');
          setProducts([]);
          setCatalogIndex({});
        }
      } finally {
        if (!isMounted) return;
        if (isBackground) {
          setIsRefreshing(false);
        } else {
          setIsLoadingProducts(false);
        }
      }
    };

    loadProducts({ eagerCache: true });

    if (CATALOG_REFRESH_INTERVAL_MS > 0) {
      refreshTimer = window.setInterval(() => {
        if (!isMounted || !isOnline) return;
        loadProducts({ background: true });
      }, CATALOG_REFRESH_INTERVAL_MS);
    }

    return () => {
      isMounted = false;
      if (refreshTimer) window.clearInterval(refreshTimer);
    };
  }, [currentUser?.tenant_id, token, isOnline]);

  const filteredProducts = useMemo(() => {
    const trimmed = query.trim().toLowerCase();
    if (!trimmed) return products;
    return products.filter((p) => {
      const nameMatch = p.name.toLowerCase().includes(trimmed);
      const skuMatch = p.sku ? p.sku.toLowerCase().includes(trimmed) : false;
      return nameMatch || skuMatch;
    });
  }, [products, query]);

  const lastSyncLabel = lastSyncTime ? formatRelativeTime(lastSyncTime) : 'No sync yet';
  const lastFetchLabel = lastFetchDuration !== null ? `${(lastFetchDuration / 1000).toFixed(1)}s` : null;
  const isCatalogStale = lastSyncTime
    ? Date.now() - lastSyncTime > Math.max(CATALOG_REFRESH_INTERVAL_MS * 2, 5 * 60 * 1000)
    : false;

  const totalUnits = useMemo(() => cart.reduce((sum, item) => sum + item.quantity, 0), [cart]);
  const cartBuildStartRef = useRef<number | null>(null);
  const now = () => (typeof performance !== 'undefined' && typeof performance.now === 'function' ? performance.now() : Date.now());

  useEffect(() => {
    if (totalUnits === 0) {
      cartBuildStartRef.current = null;
      return;
    }
    if (totalUnits === 1 && cartBuildStartRef.current === null) {
      cartBuildStartRef.current = now();
      return;
    }
    if (totalUnits >= 3 && cartBuildStartRef.current !== null) {
      const elapsed = now() - cartBuildStartRef.current;
      if (elapsed <= 10000 && import.meta.env.DEV) {
        console.info(`[Cart UX] 3 item build completed in ${(elapsed / 1000).toFixed(2)}s`);
      }
      cartBuildStartRef.current = null;
    }
  }, [totalUnits]);

  const cartDiagnostics = useMemo(() => {
    const diagnostics: Record<string, { inactive: boolean; priceChanged: boolean; latestPrice?: number }> = {};
    for (const item of cart) {
      const entry = catalogIndex[item.id];
      if (!entry) {
        diagnostics[item.id] = { inactive: true, priceChanged: false };
        continue;
      }
      diagnostics[item.id] = {
        inactive: !entry.active,
        priceChanged: Math.abs(entry.price - item.price) > 0.0001,
        latestPrice: entry.price,
      };
    }
    return diagnostics;
  }, [cart, catalogIndex]);

  const subtotal = totalAmount;
  const tax = Number((subtotal * SALES_TAX_RATE).toFixed(2));
  const displayTotal = Number((subtotal + tax).toFixed(2));

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
            onClick={() => navigate('/pos')}
          >
            POS Terminal
          </button>
          <button
            className="px-4 py-2 rounded border border-cyan-500 text-cyan-700 hover:bg-cyan-500 hover:text-white transition-colors"
            onClick={() => navigate('/history')}
          >
            Orders{queuedOrders.length > 0 ? ` ({queuedOrders.length})` : ''}
          </button>
          <button className="bg-red-500 text-white px-4 py-2 rounded hover:bg-red-600" onClick={() => { logout(); navigate('/login'); }}>Logout</button>
        </div>
      </header

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
            <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 mb-2">
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
            <div className="flex flex-wrap items-center justify-between gap-3 text-xs text-gray-500 mb-3">
              <span>Last sync: {lastSyncLabel}</span>
              <div className="flex flex-wrap items-center gap-3">
                {lastFetchLabel && <span>Last load: {lastFetchLabel}</span>}
                {isCatalogStale && <span className="text-amber-600">Catalog older than expected</span>}
                {isRefreshing && <span className="text-indigo-500">Refreshing...</span>}
              </div>
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
              id: item.id,
              name: item.name,
              price: item.price,
              quantity: item.quantity,
              onRemove: () => removeItem(item.id),
              onIncrement: () => incrementQty(item),
              onDecrement: () => decrementQty(item),
              status: cartDiagnostics[item.id],
            }))}
            subtotal={subtotal}
            tax={tax}
            total={displayTotal}
            taxRate={SALES_TAX_RATE}
            onCheckout={() => navigate('/checkout')}
            onClear={clearCart}
          />
        </main>
      </div>
    </div>
  );
};

export default SalesPage;
