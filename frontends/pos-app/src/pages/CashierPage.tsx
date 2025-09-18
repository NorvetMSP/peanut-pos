import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import { useCart } from '../CartContext';
import type { PaymentMethod, DraftOrderPayload, SubmitOrderResult } from '../OrderContext';
import { useOrders } from '../OrderContext';
import { useProducts } from '../hooks/useProducts';
import { useOfflineQueue } from '../hooks/useOfflineQueue';
import { useSyncOnReconnect } from '../hooks/useSyncOnReconnect';
import { useSubmitOrder } from '../hooks/useSubmitOrder';
import OfflineBanner from '../components/pos/OfflineBanner';
import QueuedOrdersBanner from '../components/pos/QueuedOrdersBanner';
import SearchBar from '../components/pos/SearchBar';
import CategoryFilter from '../components/pos/CategoryFilter';
import ProductGrid from '../components/pos/ProductGrid';
import CartSidebar from '../components/pos/CartSidebar';
import SubmitSalePanel from '../components/pos/SubmitSalePanel';
import RecentOrdersDrawer from '../components/pos/RecentOrdersDrawer';
import ReplaceItemModal from '../components/pos/ReplaceItemModal';

const PRODUCT_SERVICE_URL = (import.meta.env.VITE_PRODUCT_SERVICE_URL ?? 'http://localhost:8081').replace(/\/$/, '');

const CashierPage: React.FC = () => {
  const navigate = useNavigate();
  const { isLoggedIn, logout, currentUser, token } = useAuth();
  const { cart, addItem, removeItem, incrementItemQuantity, decrementItemQuantity, clearCart, totalAmount } = useCart();
  const { isOnline } = useOrders();
  const { products, categories, isLoading: isLoadingProducts, error, isOfflineResult } = useProducts();
  const { queuedOrders, isSyncing, retryQueue } = useOfflineQueue();
  const { submit: submitOrder, submitting } = useSubmitOrder();

  const [query, setQuery] = useState('');
  const [categoryFilter, setCategoryFilter] = useState('All');
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [paymentMethod, setPaymentMethod] = useState<PaymentMethod>('cash');
  const [inactiveItems, setInactiveItems] = useState<string[]>([]);
  const [pendingSubmit, setPendingSubmit] = useState(false);

  useSyncOnReconnect();

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
    };
    try {
      const result = await submitOrder(payload);
      if (!result) return;
      clearCart();
      setInactiveItems([]);
      openPaymentLink(result);
    } catch (err) {
      console.error('Order submission failed', err);
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
    <div className="min-h-screen flex flex-col bg-gray-100 dark:bg-gray-900">
      <header className="flex items-center justify-between px-6 py-4 bg-white dark:bg-gray-800 shadow-md">
        <div className="flex items-center gap-3">
          <img src="/assets/logo_transparent.png" alt="NovaPOS Logo" className="h-10 w-auto" />
          <span className="text-2xl font-bold text-gray-800 dark:text-gray-100 tracking-tight">NovaPOS</span>
        </div>
        <div className="flex items-center gap-3">
          <button
            type="button"
            className="px-4 py-2 rounded border border-cyan-500 text-cyan-700 hover:bg-cyan-500 hover:text-white transition-colors"
            onClick={() => setDrawerOpen(true)}
          >
            Orders{queuedOrders.length > 0 ? ` (${queuedOrders.length})` : ''}
          </button>
          <button
            type="button"
            className="bg-red-500 text-white px-4 py-2 rounded hover:bg-red-600"
            onClick={() => {
              logout();
              navigate('/login');
            }}
          >
            Logout
          </button>
        </div>
      </header>

      {!isOnline && <OfflineBanner queuedCount={queuedOrders.length} />}
      {isOnline && queuedOrders.length > 0 && (
        <QueuedOrdersBanner count={queuedOrders.length} syncing={isSyncing} onSync={handleManualSync} />
      )}

      <main className="flex-1 flex flex-col md:flex-row md:items-start md:justify-center px-4 py-6 gap-6">
        <section className="flex-1 max-w-3xl">
          <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 mb-4">
            <SearchBar query={query} onQueryChange={setQuery} />
            {categories.length > 1 && (
              <CategoryFilter categories={categories} selected={categoryFilter} onSelect={setCategoryFilter} />
            )}
          </div>
          {isLoadingProducts && <div className="text-sm text-gray-500 mb-2">Loading products...</div>}
          {isOfflineResult && !isLoadingProducts && (
            <div className="text-sm text-amber-600 mb-2">Offline mode: showing last synced catalog.</div>
          )}
          {error && !isLoadingProducts && <div className="text-sm text-red-600 mb-2">{error}</div>}
          <ProductGrid products={filteredProducts} onAddProduct={product => addItem({ id: product.id, name: product.name, price: product.price, sku: product.sku })} />
          {!isLoadingProducts && filteredProducts.length === 0 && (
            <div className="text-center text-gray-500 mt-4">No products found. Try adjusting your search or category filter.</div>
          )}
        </section>
        <aside className="w-full max-w-md mx-auto">
          <CartSidebar
            items={cart}
            inactiveItemIds={inactiveCartItemIds}
            onAddQty={incrementItemQuantity}
            onSubQty={decrementItemQuantity}
            onRemoveItem={removeItem}
          />
          <SubmitSalePanel
            total={totalAmount}
            paymentMethod={paymentMethod}
            onPaymentMethodChange={setPaymentMethod}
            onSubmit={handleSubmitSale}
            submitting={submitting}
          />
        </aside>
      </main>

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
