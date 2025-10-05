import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import { useCart } from '../CartContext';
import type { PaymentMethod, DraftOrderPayload, SubmitOrderResult } from '../OrderContext';
import { useOrders } from '../OrderContext';
import { useProducts } from '../hooks/useProducts'; 
import { useSyncOnReconnect } from '../hooks/useSyncOnReconnect';
import { useSubmitOrder } from '../hooks/useSubmitOrder';
import OfflineBanner from '../components/pos/OfflineBanner';
import QueuedOrdersBanner from '../components/pos/QueuedOrdersBanner';
import SearchBar from '../components/pos/SearchBar';
import CategoryFilter from '../components/pos/CategoryFilter';
import ProductGrid from '../components/pos/ProductGrid';
import CartSidebar from '../components/pos/CartSidebar';
import RecentOrdersDrawer from '../components/pos/RecentOrdersDrawer';
import ReplaceItemModal from '../components/pos/ReplaceItemModal';
import logoTransparent from '../assets/logo_transparent.png';
import './CashierPageModern.css';

const PRODUCT_SERVICE_URL = (import.meta.env.VITE_PRODUCT_SERVICE_URL ?? 'http://localhost:8081').replace(/\/$/, '');
const IDLE_EVENTS: Array<keyof WindowEventMap> = ['mousemove', 'mousedown', 'keypress', 'touchstart', 'focus'];

const formatRelativeTime = (timestamp: number): string => {
  const diff = Date.now() - timestamp;
  if (Number.isNaN(diff) || diff < 0) return 'just now';
  const seconds = Math.round(diff / 1000);
  if (seconds < 60) return `${Math.max(seconds, 1)}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return `${days}d ago`;
};

const formatIdleDuration = (seconds: number): string => {
  const minutes = Math.floor(seconds / 60) % 60;
  const hours = Math.floor(seconds / 3600);
  const secs = seconds % 60;
  if (hours > 0) {
    return `${hours.toString().padStart(2, '0')}:${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
  }
  return `${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
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

const CashierPage: React.FC = () => {
  const navigate = useNavigate();
  const { isLoggedIn, logout, currentUser, token } = useAuth();
  const { cart, addItem, removeItem, incrementItemQuantity, decrementItemQuantity, clearCart, totalAmount } = useCart();
  const { isOnline, queuedOrders, isSyncing, retryQueue, recentOrders } = useOrders();
  const { products, categories, isLoading: isLoadingProducts, error, isOfflineResult } = useProducts();
  const { submit: submitOrder, submitting } = useSubmitOrder();

  const [query, setQuery] = useState('');
  const [categoryFilter, setCategoryFilter] = useState('All');
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [paymentMethod, setPaymentMethod] = useState<PaymentMethod>('cash');
  const [inactiveItems, setInactiveItems] = useState<string[]>([]);
  const [pendingSubmit, setPendingSubmit] = useState(false);
  const [idleSeconds, setIdleSeconds] = useState(0);
  const [paymentError, setPaymentError] = useState<string | null>(null);

  useSyncOnReconnect();

  useEffect(() => {
    if (typeof window === 'undefined') return undefined;
    const tick = window.setInterval(() => {
      setIdleSeconds(prev => Math.min(prev + 1, 24 * 60 * 60));
    }, 1000);
    const reset = () => setIdleSeconds(0);
    IDLE_EVENTS.forEach(event => window.addEventListener(event, reset, { passive: true }));
    return () => {
      window.clearInterval(tick);
      IDLE_EVENTS.forEach(event => window.removeEventListener(event, reset));
    };
  }, []);

  const tenantId = useMemo(() => {
    const raw = currentUser?.tenant_id;
    if (raw === undefined || raw === null) return null;
    return String(raw);
  }, [currentUser?.tenant_id]);

  useEffect(() => {
    if (!isLoggedIn) {
      navigate('/login', { replace: true });
    }
  }, [isLoggedIn, navigate]);

  useEffect(() => {
    if (!categories.includes(categoryFilter)) {
      setCategoryFilter('All');
    }
  }, [categories, categoryFilter]);

  const storeLabel = useMemo(() => {
    const record = currentUser as Record<string, unknown> | null | undefined;
    const fromFields = extractUserField(record, ['store_name', 'store', 'location', 'branch', 'tenant_name']);
    if (fromFields) return fromFields;
    if (typeof currentUser?.tenant_id === 'string' && currentUser.tenant_id.trim().length > 0) {
      return currentUser.tenant_id;
    }
    return 'Unassigned Store';
  }, [currentUser]);

  const cashierLabel = useMemo(() => {
    const record = currentUser as Record<string, unknown> | null | undefined;
    const fromFields = extractUserField(record, ['display_name', 'name', 'full_name', 'username', 'email']);
    if (fromFields) return fromFields;
    return 'Cashier';
  }, [currentUser]);

  const filteredProducts = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    const normalizedCategory = categoryFilter.trim().toLowerCase();
    return products.filter(product => {
      const matchesQuery =
        normalizedQuery.length === 0 ||
        product.name.toLowerCase().includes(normalizedQuery) ||
        (product.sku ?? '').toLowerCase().includes(normalizedQuery);
      const matchesCategory =
        categoryFilter === 'All' ||
        (product.category && product.category.toLowerCase() === normalizedCategory) ||
        (product.description ?? '').toLowerCase().includes(normalizedCategory);
      return matchesQuery && matchesCategory;
    });
  }, [categoryFilter, products, query]);

  const inactiveCartItemIds = useMemo(
    () => cart.filter(item => !products.some(product => product.id === item.id)).map(item => item.id),
    [cart, products],
  );

  const currentInactiveId = inactiveItems.length > 0 ? inactiveItems[0] : null;
  const currentInactiveItem = currentInactiveId ? cart.find(item => item.id === currentInactiveId) ?? null : null;

  const lastSyncedAt = useMemo(() => {
    const timestamps = recentOrders
      .map(order => order.syncedAt ?? order.createdAt)
      .filter((value): value is number => typeof value === 'number' && Number.isFinite(value));
    if (timestamps.length === 0) return null;
    return Math.max(...timestamps);
  }, [recentOrders]);

  const syncStatusLabel = isOnline
    ? lastSyncedAt
      ? `Synced ${formatRelativeTime(lastSyncedAt)}`
      : 'Synced just now'
    : 'Offline mode';

  const queueStatusLabel = isOnline
    ? queuedOrders.length === 0
      ? 'Queue empty'
      : `${queuedOrders.length} queued for sync`
    : `${queuedOrders.length} offline`;

  const openPaymentLink = (result: SubmitOrderResult) => {
    if (result.status !== 'submitted') return;
    const url = result.payment?.paymentUrl ?? result.payment?.payment_url;
    if (url) {
      window.open(url, '_blank');
    }
  };

  const triggerOrderSubmission = useCallback(async () => {
    if (cart.length === 0) return;
    const itemsPayload: DraftOrderPayload['items'] = cart.map(item => ({
      product_id: item.id,
      quantity: item.quantity,
      unit_price: item.price,
      line_total: Number((item.price * item.quantity).toFixed(2)),
    }));
    const payload: DraftOrderPayload = {
      items: itemsPayload,
      payment_method: paymentMethod,
      total: Number(totalAmount.toFixed(2)),
      metadata: paymentError ? { retry: Date.now() } : undefined,
    };
    try {
      const result = await submitOrder(payload);
      if (!result) return;
      // If payment failed (e.g., card declined), surface error and allow retry without clearing cart
      if (result.status === 'submitted' && result.paymentError) {
        setPaymentError(`Payment error: ${result.paymentError}`);
        return;
      }
      // Success path: clear any prior error, clear cart, and proceed
      setPaymentError(null);
      clearCart();
      setInactiveItems([]);
      openPaymentLink(result);
    } catch (err) {
      console.error('Order submission failed', err);
      setPaymentError(err instanceof Error ? err.message : 'Order submission failed');
    } finally {
      setPendingSubmit(false);
    }
  }, [cart, clearCart, paymentMethod, submitOrder, totalAmount]);

  const handleSubmitSale = useCallback(async () => {
    if (cart.length === 0) return;

    try {
      const headers: Record<string, string> = {};
      if (tenantId) headers['X-Tenant-ID'] = tenantId;
      if (token) headers.Authorization = `Bearer ${token}`;

      const response = await fetch(`${PRODUCT_SERVICE_URL}/products`, { headers });
      let productList: Array<Record<string, unknown>> = [];
      if (response.ok) {
        const data = await response.json();
        productList = Array.isArray(data) ? data : [];
      } else if (tenantId) {
        const cached = window.localStorage.getItem(`productCache:${tenantId}`);
        if (cached) {
          try {
            const parsed = JSON.parse(cached);
            productList = Array.isArray(parsed) ? parsed : [];
          } catch {
            productList = [];
          }
        }
      }

      const inactiveIds: string[] = [];
      const activeMap = new Map<string, boolean>();
      for (const entry of productList) {
        if (!entry || typeof entry !== 'object') continue;
        const record = entry as Record<string, unknown>;
        const id = record.id;
        const active = record.active !== false;
        if (typeof id === 'string') {
          activeMap.set(id, active);
        } else if (typeof id === 'number') {
          activeMap.set(String(id), active);
        }
      }
      if (activeMap.size > 0) {
        for (const item of cart) {
          if (activeMap.get(item.id) === false) {
            inactiveIds.push(item.id);
          }
        }
      }
      if (inactiveIds.length > 0) {
        setInactiveItems(inactiveIds);
        setPendingSubmit(true);
        return;
      }
    } catch (err) {
      console.error('Error validating products before submission', err);
    }

    triggerOrderSubmission().catch(err => console.error('Unable to submit order', err));
  }, [cart, tenantId, token, triggerOrderSubmission]);

  useEffect(() => {
    if (pendingSubmit && inactiveItems.length === 0) {
      triggerOrderSubmission().catch(err => console.error('Unable to submit order', err));
    }
  }, [inactiveItems.length, pendingSubmit, triggerOrderSubmission]);

  const handleReplaceItem = useCallback(
    (oldItemId: string, replacementProductId: string) => {
      const oldItem = cart.find(item => item.id === oldItemId);
      const replacement = products.find(product => product.id === replacementProductId);
      if (!oldItem || !replacement) return;
      const quantity = oldItem.quantity;
      removeItem(oldItemId);
      for (let i = 0; i < quantity; i += 1) {
        addItem({ id: replacement.id, name: replacement.name, price: replacement.price, sku: replacement.sku });
      }
      setInactiveItems(prev => prev.filter(id => id !== oldItemId));
    },
    [addItem, cart, products, removeItem],
  );

  const handleRemoveInactiveItem = useCallback(
    (itemId: string) => {
      removeItem(itemId);
      setInactiveItems(prev => prev.filter(id => id !== itemId));
    },
    [removeItem],
  );

  const handleCancelReplace = useCallback(() => {
    setInactiveItems([]);
    setPendingSubmit(false);
  }, []);

  const handleManualSync = useCallback(() => {
    retryQueue().catch(err => console.warn('Manual queue retry failed', err));
  }, [retryQueue]);

  return (
    <div className="cashier-root">
      <header className="cashier-header">
        <div className="cashier-header__inner">
          <div className="cashier-header__top">
            <div className="cashier-brand">
              <img src={logoTransparent} alt="NovaPOS Logo" />
              <div>
                <div className="cashier-brand__title">NovaPOS</div>
                <div className="cashier-status-list">
                  <span className="cashier-badge cashier-badge--muted">Store: {storeLabel}</span>
                  <span className="cashier-badge cashier-badge--muted">Cashier: {cashierLabel} | Idle {formatIdleDuration(idleSeconds)}</span>
                  <span
                    className={`cashier-badge ${isOnline ? 'cashier-badge--online' : 'cashier-badge--offline'}`}
                  >
                    {isOnline ? 'Online' : 'Offline'}
                  </span>
                  <span className="cashier-badge cashier-badge--muted">{syncStatusLabel}</span>
                </div>
              </div>
            </div>
            <div className="cashier-header__actions">
              <button
                type="button"
                className="cashier-button cashier-button--secondary"
                onClick={handleManualSync}
                disabled={!isOnline || queuedOrders.length === 0 || isSyncing}
              >
                {isSyncing ? 'Syncing...' : 'Sync Now'}
              </button>
              <button
                type="button"
                className="cashier-button cashier-button--danger"
                onClick={() => {
                  logout();
                  navigate('/login');
                }}
              >
                Logout
              </button>
            </div>
          </div>
          <div className="cashier-header__controls">
            <SearchBar query={query} onQueryChange={setQuery} />
            {categories.length > 1 && (
              <CategoryFilter categories={categories} selected={categoryFilter} onSelect={setCategoryFilter} />
            )}
            <button type="button" className="cashier-button cashier-recent-orders" onClick={() => setDrawerOpen(true)}>
              Recent Orders (F2)
              {queuedOrders.length > 0 && (
                <span className="cashier-recent-orders__badge">{queuedOrders.length}</span>
              )}
            </button>
          </div>
        </div>
      </header>

      {!isOnline && <OfflineBanner queuedCount={queuedOrders.length} />}
      {isOnline && queuedOrders.length > 0 && (
        <QueuedOrdersBanner count={queuedOrders.length} syncing={isSyncing} onSync={handleManualSync} />
      )}

      <main className="cashier-main">
        <div className="cashier-main__grid">
          <section className="cashier-card cashier-card--catalog">
            <div className="cashier-card__header">
              <div>
                <h2 className="cashier-card__title">Product Catalog</h2>
                <p className="cashier-card__subtitle">Search, browse, and add products to the active cart.</p>
              </div>
              <span className="cashier-card__indicator">
                {isLoadingProducts ? 'Loading...' : `${filteredProducts.length} items`}
              </span>
            </div>
            {error && !isLoadingProducts && (
              <div className="cashier-card__notice cashier-card__notice--error">{error}</div>
            )}
            {paymentError && (
              <div className="cashier-card__notice cashier-card__notice--error">{paymentError}</div>
            )}
            {isOfflineResult && !isLoadingProducts && (
              <div className="cashier-card__notice cashier-card__notice--offline">
                Offline mode: showing last synced catalog.
              </div>
            )}
            <ProductGrid
              products={filteredProducts}
              onAddProduct={product =>
                addItem({ id: product.id, name: product.name, price: product.price, sku: product.sku })
              }
            />
            {!isLoadingProducts && filteredProducts.length === 0 && (
              <div className="cashier-card__empty-state">
                No products found. Try adjusting your search or category filter.
              </div>
            )}
          </section>

          <aside className="cashier-sidebar">
            <CartSidebar
              items={cart}
              inactiveItemIds={inactiveCartItemIds}
              onAddQty={incrementItemQuantity}
              onSubQty={decrementItemQuantity}
              onRemoveItem={removeItem}
            />
            <div className="cashier-cart-payment">
              <div>
                <div className="cashier-cart-payment__title">Payment Method</div>
                <div className="cashier-cart-payment__subtitle">Select tender type for this sale</div>
              </div>
              <select
                value={paymentMethod}
                onChange={event => setPaymentMethod(event.target.value as PaymentMethod)}
                className="cashier-select cashier-cart-payment__select"
                aria-label="Select payment method"
              >
                <option value="cash">Cash</option>
                <option value="card">Card</option>
                <option value="crypto">Crypto</option>
              </select>
            </div>
          </aside>
        </div>
      </main>

      <footer className="cashier-footer">
        <div className="cashier-footer__inner">
          <div className="cashier-footer__left">
            <button
              type="button"
              className="cashier-footer__clear"
              onClick={clearCart}
              disabled={cart.length === 0}
            >
              Clear Cart
            </button>
            <span className="cashier-footer__queue">Queue: {queueStatusLabel}</span>
          </div>
          <div className="cashier-footer__right">
            <div className="cashier-footer__total">
              <label>Total</label>
              <strong>${totalAmount.toFixed(2)}</strong>
            </div>
            <button
              type="button"
              className="cashier-footer__submit"
              onClick={handleSubmitSale}
              disabled={cart.length === 0 || submitting}
            >
              {submitting ? 'Processing...' : 'Submit / Complete Sale'}
              <span className="cashier-footer__shortcut">(Enter / F7)</span>
            </button>
          </div>
        </div>
      </footer>

      <RecentOrdersDrawer open={drawerOpen} onClose={() => setDrawerOpen(false)} />

      {currentInactiveItem && (
        <ReplaceItemModal
          item={currentInactiveItem}
          products={products}
          onReplace={replacementId => handleReplaceItem(currentInactiveItem.id, replacementId)}
          onRemove={() => handleRemoveInactiveItem(currentInactiveItem.id)}
          onCancel={handleCancelReplace}
        />
      )}
    </div>
  );
};

export default CashierPage;
