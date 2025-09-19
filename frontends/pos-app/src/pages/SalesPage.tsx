import React, { useEffect, useMemo, useRef, useState } from 'react';
import logoTransparent from '../assets/logo_transparent.png';
import productIcon from '../assets/react.svg';
import CartCard from '../components/CartCard';
import './SalesPage.css';
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

const extractUserField = (user: Record<string, unknown> | null | undefined, keys: string[]): string | null => {
  if (!user) return null;
  for (const key of keys) {
    const value = user[key];
    if (typeof value === 'string' && value.trim().length > 0) {
      return value.trim();
    }
  }
  return null;
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
    return products.filter(product => {
      const nameMatch = product.name.toLowerCase().includes(trimmed);
      const skuMatch = product.sku ? product.sku.toLowerCase().includes(trimmed) : false;
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

  const queuedOrdersCount = queuedOrders.length;

  const storeLabel = useMemo(() => {
    const record = currentUser as Record<string, unknown> | null | undefined;
    const fromFields = extractUserField(record, ['store_name', 'store', 'location', 'branch', 'tenant_name']);
    if (fromFields) return fromFields;
    if (typeof currentUser?.tenant_id === 'string' && currentUser.tenant_id.trim().length > 0) {
      return currentUser.tenant_id;
    }
    return 'Unassigned Store';
  }, [currentUser]);

  const userLabel = useMemo(() => {
    const record = currentUser as Record<string, unknown> | null | undefined;
    const fromFields = extractUserField(record, ['display_name', 'name', 'full_name', 'username', 'email']);
    if (fromFields) return fromFields;
    return 'Team Member';
  }, [currentUser]);

  const queueStatusLabel = useMemo(() => {
    if (isOnline) {
      if (queuedOrdersCount === 0) return 'Queue empty';
      return `${queuedOrdersCount} queued`;
    }
    if (queuedOrdersCount === 0) return 'No offline orders';
    return `${queuedOrdersCount} offline`;
  }, [isOnline, queuedOrdersCount]);

  const syncStatusLabel = useMemo(() => {
    if (!isOnline) return 'Offline mode';
    if (!lastSyncTime) return 'Synced just now';
    return `Synced ${formatRelativeTime(lastSyncTime)}`;
  }, [isOnline, lastSyncTime]);

  const totalProductCount = products.length;
  const filteredProductCount = filteredProducts.length;

  const productCountLabel = useMemo(() => {
    if (totalProductCount === 0) return 'No products';
    if (filteredProductCount === totalProductCount) {
      return `${totalProductCount} items`;
    }
    return `${filteredProductCount} of ${totalProductCount} items`;
  }, [filteredProductCount, totalProductCount]);

  const cartItemCount = cart.length;
  const cartUnitCount = totalUnits;

  const incrementQty = (item: { id: string }) => {
    incrementItemQuantity(item.id);
  };
  const decrementQty = (item: { id: string; quantity: number }) => {
    if (item.quantity > 1) {
      decrementItemQuantity(item.id);
    }
  };

  const handleAddToCart = (product: ServiceProduct) => {
    addItem({
      id: product.id,
      name: product.name,
      price: product.price,
      sku: product.sku ?? undefined,
    });
  };

  return (
    <div className="sales-root">
      <header className="sales-header">
        <div className="sales-header__inner">
          <div className="sales-header__top">
            <div className="sales-brand">
              <img src={logoTransparent} alt="NovaPOS Logo" />
              <div>
                <div className="sales-brand__title">NovaPOS Catalog</div>
                <div className="sales-status-list">
                  <span className="sales-badge sales-badge--muted">Store: {storeLabel}</span>
                  <span className="sales-badge sales-badge--muted">User: {userLabel}</span>
                  <span
                    className={`sales-badge ${isOnline ? 'sales-badge--online' : 'sales-badge--offline'}`}
                  >
                    {isOnline ? 'Online' : 'Offline'}
                  </span>
                  <span className="sales-badge sales-badge--muted">{syncStatusLabel}</span>
                </div>
              </div>
            </div>
            <div className="sales-header__actions">
              <button
                type="button"
                className="sales-button sales-button--light"
                onClick={() => navigate('/pos')}
              >
                POS Terminal
              </button>
              <button
                type="button"
                className="sales-button sales-button--light"
                onClick={() => navigate('/history')}
              >
                Orders
                {queuedOrdersCount > 0 && <span className="sales-orders-badge">{queuedOrdersCount}</span>}
              </button>
              <button
                type="button"
                className="sales-button sales-button--danger"
                onClick={() => {
                  logout();
                  navigate('/login');
                }}
              >
                Logout
              </button>
            </div>
          </div>
          <div className="sales-header__controls">
            <div className="sales-field">
              <label className="sales-field__label" htmlFor="catalog-search">
                Search catalog
              </label>
              <input
                id="catalog-search"
                type="search"
                value={query}
                onChange={event => setQuery(event.target.value)}
                placeholder="Search products by name or SKU"
                className="sales-input"
                aria-label="Search products by name or SKU"
              />
            </div>
            <div className="sales-meta">
              <span className="sales-meta__item">Catalog: {productCountLabel}</span>
              <span className="sales-meta__item">Last sync: {lastSyncLabel}</span>
              {lastFetchLabel && <span className="sales-meta__item">Fetch time: {lastFetchLabel}</span>}
              {isCatalogStale && (
                <span className="sales-meta__item sales-meta__item--warning">Catalog older than expected</span>
              )}
              {isRefreshing && <span className="sales-meta__item sales-meta__item--info">Refreshing...</span>}
            </div>
          </div>
        </div>
      </header>

      {!isOnline && (
        <div className="sales-banner sales-banner--offline">
          Offline mode - {queuedOrdersCount} order{queuedOrdersCount === 1 ? '' : 's'} queued. Sales will sync automatically once reconnected.
        </div>
      )}
      {isOnline && queuedOrdersCount > 0 && (
        <div className="sales-banner sales-banner--queue">
          {isSyncing ? 'Synchronizing queued orders...' : `${queuedOrdersCount} order${queuedOrdersCount === 1 ? '' : 's'} awaiting sync.`}
        </div>
      )}

      <main className="sales-main">
        <div className="sales-main__grid">
          <section className="sales-card sales-card--catalog">
            <div className="sales-card__header">
              <div>
                <h2 className="sales-card__title">Product Catalog</h2>
                <p className="sales-card__subtitle">Browse and curate the store offerings.</p>
              </div>
              <span className="sales-card__indicator">
                {isLoadingProducts ? 'Loading...' : productCountLabel}
              </span>
            </div>
            {isLoadingProducts && (
              <div className="sales-card__notice sales-card__notice--info">Loading products...</div>
            )}
            {error && !isLoadingProducts && (
              <div className="sales-card__notice sales-card__notice--error">{error}</div>
            )}
            {isOfflineResult && !isLoadingProducts && (
              <div className="sales-card__notice sales-card__notice--offline">
                Offline mode: showing last synced catalog.
              </div>
            )}
            {isCatalogStale && !isLoadingProducts && (
              <div className="sales-card__notice sales-card__notice--warning">
                Catalog appears out of date. Try refreshing soon.
              </div>
            )}
            <div className="sales-product-grid">
              {filteredProducts.map(product => (
                <div key={product.id} className="sales-product-card">
                  <div className="sales-product-card__image">
                    {product.image_url ? (
                      <img src={product.image_url} alt={product.name} />
                    ) : (
                      <div className="sales-product-card__placeholder">
                        <img src={FALLBACK_IMAGE} alt="" aria-hidden="true" />
                        <span>No image</span>
                      </div>
                    )}
                  </div>
                  <div className="sales-product-card__body">
                    <div className="sales-product-card__header">
                      <span className="sales-product-card__name">{product.name}</span>
                      <span className="sales-product-card__price">${product.price.toFixed(2)}</span>
                    </div>
                    {product.sku && <span className="sales-product-card__sku">SKU: {product.sku}</span>}
                    <p
                      className={`sales-product-card__description${
                        product.description ? '' : ' sales-product-card__description--muted'
                      }`}
                    >
                      {product.description || 'No description provided.'}
                    </p>
                    <div className="sales-product-card__footer">
                      <button
                        type="button"
                        className="sales-product-card__add"
                        onClick={() => handleAddToCart(product)}
                        aria-label={`Add ${product.name} to cart`}
                      >
                        Add to Cart
                      </button>
                    </div>
                  </div>
                </div>
              ))}
            </div>
            {!isLoadingProducts && filteredProducts.length === 0 && (
              <div className="sales-card__empty-state">
                No products found. Try adjusting your search or filters.
              </div>
            )}
          </section>

          <aside className="sales-sidebar">
            <div className="sales-card sales-card--snapshot">
              <h3 className="sales-card__title">Cart Snapshot</h3>
              <div className="sales-snapshot-grid">
                <div>
                  <span className="sales-snapshot__label">Items</span>
                  <span className="sales-snapshot__value">{cartItemCount}</span>
                </div>
                <div>
                  <span className="sales-snapshot__label">Units</span>
                  <span className="sales-snapshot__value">{cartUnitCount}</span>
                </div>
                <div>
                  <span className="sales-snapshot__label">Total</span>
                  <span className="sales-snapshot__value">${displayTotal.toFixed(2)}</span>
                </div>
                <div>
                  <span className="sales-snapshot__label">Queue</span>
                  <span className="sales-snapshot__badge">{queueStatusLabel}</span>
                </div>
              </div>
            </div>
            <div className="sales-cart-shell">
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
            </div>
          </aside>
        </div>
      </main>
    </div>
  );
};

export default SalesPage;
