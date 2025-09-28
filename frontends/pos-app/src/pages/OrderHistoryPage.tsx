import React from 'react';
import { useNavigate } from 'react-router-dom';
import { useOrders } from '../OrderContext';
import type { PaymentMethod } from '../OrderContext';

const formatTimestamp = (value: number): string => {
  try {
    return new Date(value).toLocaleString();
  } catch {
    return '-';
  }
};

type StatusFilter = 'all' | 'queued' | 'pending' | 'completed' | 'errored';

type RecentOrder = ReturnType<typeof useOrders>['recentOrders'][number];
const FILTER_STORAGE_KEY = 'pos-order-history-filters';

const categorizeStatus = (order: RecentOrder): StatusFilter => {
  if (order.offline) return 'queued';
  const normalizedStatus = order.status.toLowerCase();
  const normalizedPayment = (order.paymentStatus ?? '').toLowerCase();

  if (normalizedStatus.includes('queue')) return 'queued';
  if (
    normalizedStatus.includes('cancel') ||
    normalizedStatus.includes('void') ||
    normalizedStatus.includes('fail') ||
    normalizedStatus.includes('declin') ||
    normalizedPayment.includes('fail') ||
    normalizedPayment.includes('declin') ||
    normalizedPayment.includes('error')
  ) {
    return 'errored';
  }
  if (
    normalizedStatus.includes('complete') ||
    normalizedStatus.includes('accepted') ||
    normalizedStatus.includes('fulfilled') ||
    normalizedStatus.includes('success') ||
    normalizedPayment.includes('paid') ||
    normalizedPayment.includes('settled') ||
    normalizedPayment.includes('captured') ||
    normalizedPayment.includes('success')
  ) {
    return 'completed';
  }
  return 'pending';
};

const matchesSearch = (haystack: string, needle: string) => haystack.toLowerCase().includes(needle);

const OrderHistoryPage: React.FC = () => {
  const { queuedOrders, recentOrders, isOnline, isSyncing, retryQueue, refreshOrderStatuses } = useOrders();
  const navigate = useNavigate();

  const readStoredFilters = React.useCallback(() => {
    const defaults = { search: '', status: 'all' as StatusFilter, payment: 'all' as 'all' | PaymentMethod };
    if (typeof window === 'undefined') return defaults;
    try {
      const raw = window.localStorage.getItem(FILTER_STORAGE_KEY);
      if (!raw) return defaults;
      const parsed = JSON.parse(raw);
      const isStatus = (value: unknown): value is StatusFilter => value === 'all' || value === 'queued' || value === 'pending' || value === 'completed' || value === 'errored';
      const isPayment = (value: unknown): value is 'all' | PaymentMethod => value === 'all' || value === 'card' || value === 'cash' || value === 'crypto';
      return {
        search: typeof parsed.search === 'string' ? parsed.search : defaults.search,
        status: isStatus(parsed.status) ? parsed.status : defaults.status,
        payment: isPayment(parsed.payment) ? parsed.payment : defaults.payment,
      };
    } catch (err) {
      console.warn('Unable to read stored history filters', err);
      return defaults;
    }
  }, []);

  const storedFilters = React.useMemo(() => readStoredFilters(), [readStoredFilters]);

  const [searchTerm, setSearchTerm] = React.useState(storedFilters.search);
  const [statusFilter, setStatusFilter] = React.useState<StatusFilter>(storedFilters.status);
  const [paymentFilter, setPaymentFilter] = React.useState<'all' | PaymentMethod>(storedFilters.payment);
  const [isRefreshing, setIsRefreshing] = React.useState(false);

  const normalizedSearch = searchTerm.trim().toLowerCase();

  const filteredQueuedOrders = React.useMemo(() => {
    if (!normalizedSearch) return queuedOrders;
    return queuedOrders.filter(entry => {
      const haystack = `${entry.tempId} ${entry.payload.payment_method} ${entry.payload.total} ${entry.lastError ?? ''}`.toLowerCase();
      return matchesSearch(haystack, normalizedSearch);
    });
  }, [normalizedSearch, queuedOrders]);

  const filteredRecentOrders = React.useMemo(() => {
    return recentOrders.filter(order => {
      if (statusFilter !== 'all' && categorizeStatus(order) !== statusFilter) {
        return false;
      }
      if (paymentFilter !== 'all' && order.paymentMethod !== paymentFilter) {
        return false;
      }
      if (!normalizedSearch) return true;
      const haystack = `${order.reference} ${order.status} ${order.paymentStatus ?? ''} ${order.paymentMethod} ${order.note ?? ''}`.toLowerCase();
      return matchesSearch(haystack, normalizedSearch);
    });
  }, [normalizedSearch, paymentFilter, recentOrders, statusFilter]);

  const handleRetry = React.useCallback(() => {
    retryQueue().catch(err => console.warn('Retry failed', err));
  }, [retryQueue]);

  const handleRefreshStatuses = React.useCallback(() => {
    setIsRefreshing(true);
    refreshOrderStatuses()
      .catch(err => console.warn('Status refresh failed', err))
      .finally(() => setIsRefreshing(false));
  }, [refreshOrderStatuses]);

  const paymentOptions: Array<'all' | PaymentMethod> = ['all', 'card', 'cash', 'crypto'];

  return (
    <div className="min-h-screen bg-gray-100 dark:bg-gray-900 text-gray-900 dark:text-gray-100">
      <div className="max-w-5xl mx-auto px-4 py-8">
        <header className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 mb-8">
          <div>
            <h1 className="text-3xl font-bold">Order History</h1>
            <p className="text-sm text-gray-600 dark:text-gray-400">Review submitted and queued sales.</p>
          </div>
          <div className="flex flex-wrap gap-3">
            <button
              className="px-4 py-2 rounded border border-cyan-500 text-cyan-700 hover:bg-cyan-500 hover:text-white transition-colors"
              onClick={() => navigate('/pos')}
            >
              POS Terminal
            </button>
            <button
              className="px-4 py-2 rounded bg-gray-200 dark:bg-gray-800 hover:bg-gray-300 dark:hover:bg-gray-700"
              onClick={() => navigate('/sales')}
            >
              Back to Sales
            </button>
            <button
              className="px-4 py-2 rounded border border-cyan-500 text-cyan-700 hover:bg-cyan-500 hover:text-white transition-colors"
              onClick={handleRefreshStatuses}
              disabled={!isOnline || isRefreshing}
            >
              {isRefreshing ? 'Refreshing...' : 'Refresh Status'}
            </button>
            <button
              className="px-4 py-2 rounded bg-primary text-white"
              style={{ backgroundColor: '#19b4b9' }}
              disabled={!isOnline || isSyncing || queuedOrders.length === 0}
              onClick={handleRetry}
            >
              {isSyncing ? 'Syncing...' : 'Retry Sync'}
            </button>
          </div>
        </header>

        {!isOnline && (
          <div className="mb-6 rounded border border-amber-400 bg-amber-100 text-amber-800 px-4 py-3">
            Offline mode - {queuedOrders.length} order{queuedOrders.length === 1 ? '' : 's'} queued.
          </div>
        )}

        {isOnline && queuedOrders.length > 0 && (
          <div className="mb-6 rounded border border-sky-400 bg-sky-100 text-sky-800 px-4 py-3">
            {isSyncing ? 'Synchronizing queued orders...' : `${queuedOrders.length} order${queuedOrders.length === 1 ? '' : 's'} awaiting sync.`}
          </div>
        )}

        <div className="grid gap-4 md:grid-cols-3 md:items-end mb-8">
          <div className="md:col-span-1">
            <label className="block text-sm font-semibold mb-1" htmlFor="history-search">Search</label>
            <input
              id="history-search"
              type="search"
              value={searchTerm}
              onChange={event => setSearchTerm(event.target.value)}
              placeholder="Reference, status, note..."
              className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-primary"
            />
          </div>
          <div className="md:col-span-1">
            <label className="block text-sm font-semibold mb-1" htmlFor="status-filter">Status</label>
            <select
              id="status-filter"
              value={statusFilter}
              onChange={event => setStatusFilter(event.target.value as StatusFilter)}
              className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-primary"
            >
              <option value="all">All statuses</option>
              <option value="queued">Queued</option>
              <option value="pending">Pending</option>
              <option value="completed">Completed</option>
              <option value="errored">Errored</option>
            </select>
          </div>
          <div className="md:col-span-1">
            <label className="block text-sm font-semibold mb-1" htmlFor="payment-filter">Payment Method</label>
            <select
              id="payment-filter"
              value={paymentFilter}
              onChange={event => setPaymentFilter(event.target.value as 'all' | PaymentMethod)}
              className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-primary"
            >
              {paymentOptions.map(option => (
                <option key={option} value={option}>
                  {option === 'all' ? 'All payment methods' : option.toUpperCase()}
                </option>
              ))}
            </select>
          </div>
        </div>

        <section className="mb-10">
          <h2 className="text-xl font-semibold mb-4">Queued Orders</h2>
          {filteredQueuedOrders.length === 0 ? (
            <p className="text-sm text-gray-600 dark:text-gray-400">No offline orders pending.</p>
          ) : (
            <div className="overflow-x-auto rounded-lg shadow bg-white dark:bg-gray-800">
              <table className="min-w-full text-sm">
                <thead className="bg-gray-200 dark:bg-gray-700 text-left">
                  <tr>
                    <th className="px-4 py-2">Temp ID</th>
                    <th className="px-4 py-2">Payment</th>
                    <th className="px-4 py-2">Total</th>
                    <th className="px-4 py-2">Attempts</th>
                    <th className="px-4 py-2">Created</th>
                    <th className="px-4 py-2">Last Error</th>
                  </tr>
                </thead>
                <tbody>
                  {filteredQueuedOrders.map(entry => (
                    <tr key={entry.tempId} className="border-t border-gray-200 dark:border-gray-700">
                      <td className="px-4 py-2 font-mono text-xs">{entry.tempId}</td>
                      <td className="px-4 py-2 uppercase">{entry.payload.payment_method}</td>
                      <td className="px-4 py-2">${entry.payload.total.toFixed(2)}</td>
                      <td className="px-4 py-2">{entry.attempts}</td>
                      <td className="px-4 py-2">{formatTimestamp(entry.createdAt)}</td>
                      <td className="px-4 py-2 text-xs text-red-600 dark:text-red-400">{entry.lastError ?? '-'}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>

        <section>
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-xl font-semibold">Recent Activity</h2>
            <span className="text-xs text-gray-500 dark:text-gray-400">Showing {filteredRecentOrders.length} of {recentOrders.length}</span>
          </div>
          {filteredRecentOrders.length === 0 ? (
            <p className="text-sm text-gray-600 dark:text-gray-400">No orders match the current filters.</p>
          ) : (
            <div className="space-y-4">
              {filteredRecentOrders.map(order => (
                <div key={order.reference} className="rounded-lg bg-white dark:bg-gray-800 shadow px-4 py-3">
                  <div className="flex flex-wrap items-center justify-between gap-3">
                    <div>
                      <div className="font-semibold">Reference: {order.reference}</div>
                      <div className="text-xs text-gray-500 dark:text-gray-400">Created {formatTimestamp(order.createdAt)}</div>
                    </div>
                    <div className="text-right">
                      <div className="text-sm">Status: {order.status}</div>
                      {order.paymentStatus && <div className="text-xs text-gray-500 dark:text-gray-400">Payment: {order.paymentStatus}</div>}
                    </div>
                  </div>
                  <div className="mt-3 text-sm text-gray-600 dark:text-gray-300 flex flex-wrap gap-4">
                    <span>Payment Method: {order.paymentMethod.toUpperCase()}</span>
                    <span>Total: ${order.total.toFixed(2)}</span>
                    {order.offline && <span className="text-amber-600">Queued offline</span>}
                    {!order.offline && order.syncedAt && (
                      <span className="text-gray-500 dark:text-gray-400">Updated {formatTimestamp(order.syncedAt)}</span>
                    )}
                  </div>
                  {order.paymentUrl && (
                    <div className="mt-2 text-xs">
                      <span className="font-semibold">Payment URL:</span>{' '}
                      <a className="text-primary" href={order.paymentUrl} target="_blank" rel="noreferrer">{order.paymentUrl}</a>
                    </div>
                  )}
                  {order.note && (
                    <div className="mt-2 text-xs text-red-600 dark:text-red-400">{order.note}</div>
                  )}
                  {order.items.length > 0 && (
                    <div className="mt-3 text-xs text-gray-600 dark:text-gray-300">
                      <div className="font-semibold">Line Items</div>
                      <ul className="mt-1 space-y-1">
                        {order.items.map((item, index) => (
                          <li key={`${order.reference}-item-${index}`} className="flex justify-between">
                            <span>{item.quantity} x {item.product_id}</span>
                            <span>${item.line_total.toFixed(2)}</span>
                          </li>
                        ))}
                      </ul>
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  );
};

export default OrderHistoryPage;




