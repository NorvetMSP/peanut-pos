import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import { resolveServiceUrl } from '../utils/env';

const ORDER_SERVICE_URL = resolveServiceUrl('VITE_ORDER_SERVICE_URL', 'http://localhost:8084');
const PAGE_SIZE = 25;

interface ReturnSummaryRecord {
  id: string;
  order_id: string;
  total: number;
  reason?: string | null;
  created_at: string;
  store_id?: string | null;
}

interface OrderLineItemRecord {
  product_id: string;
  product_name?: string | null;
  quantity: number;
  returned_quantity: number;
  unit_price: number;
  line_total: number;
}

interface OrderRecord {
  id: string;
  status: string;
  total: number;
  payment_method: string;
  created_at: string;
  store_id?: string | null;
  customer_name?: string | null;
  customer_email?: string | null;
}

interface OrderDetailRecord {
  order: OrderRecord;
  items: OrderLineItemRecord[];
}

interface ReturnFilters {
  orderId: string;
  storeId: string;
  startDate: string;
  endDate: string;
}

const defaultFilters: ReturnFilters = {
  orderId: '',
  storeId: '',
  startDate: '',
  endDate: '',
};

const ReturnsPage: React.FC = () => {
  const { token, currentUser, isLoggedIn } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();

  const [historyFilters, setHistoryFilters] = useState<ReturnFilters>(defaultFilters);
  const [returns, setReturns] = useState<ReturnSummaryRecord[]>([]);
  const [historyPage, setHistoryPage] = useState(0);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [historyHasNext, setHistoryHasNext] = useState(false);

  const [lookupOrderId, setLookupOrderId] = useState('');
  const [orderDetail, setOrderDetail] = useState<OrderDetailRecord | null>(null);
  const [orderLoading, setOrderLoading] = useState(false);
  const [orderError, setOrderError] = useState<string | null>(null);

  const [returnQuantities, setReturnQuantities] = useState<Record<string, number>>({});
  const [returnReason, setReturnReason] = useState('');
  const [submitLoading, setSubmitLoading] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitSuccess, setSubmitSuccess] = useState<string | null>(null);

  const tenantId = useMemo(() => (currentUser?.tenant_id ? String(currentUser.tenant_id) : null), [currentUser]);

  useEffect(() => {
    if (!isLoggedIn) {
      void navigate('/login', { replace: true });
    }
  }, [isLoggedIn, navigate]);

  const ensureTenant = useCallback(() => {
    if (!tenantId) {
      setHistoryError('Unable to determine tenant context.');
      return false;
    }
    return true;
  }, [tenantId]);

  const buildHeaders = useCallback(
    (isJson = true): Record<string, string> => {
      const headers: Record<string, string> = {};
      if (tenantId) headers['X-Tenant-ID'] = tenantId;
      if (token) headers['Authorization'] = `Bearer ${token}`;
      headers['Accept'] = isJson ? 'application/json' : 'text/plain';
      if (isJson) headers['Content-Type'] = 'application/json';
      return headers;
    },
    [tenantId, token]
  );

  const loadOrderDetail = useCallback(
    async (orderId: string) => {
      if (!ensureTenant()) return;
      setOrderLoading(true);
      setOrderError(null);
      setOrderDetail(null);
      setReturnQuantities({});
      setReturnReason('');
      setSubmitError(null);
      setSubmitSuccess(null);

      try {
        const response = await fetch(`${ORDER_SERVICE_URL}/orders/${orderId}`, {
          method: 'GET',
          headers: buildHeaders(),
        });
        if (!response.ok) {
          if (response.status === 404) {
            throw new Error('Order not found for this tenant.');
          }
          throw new Error(`Failed to load order (${response.status})`);
        }
        const payload = (await response.json()) as unknown;
        if (
          typeof payload !== 'object' ||
          payload === null ||
          typeof (payload as Record<string, unknown>).order !== 'object' ||
          typeof (payload as Record<string, unknown>).items === 'undefined'
        ) {
          throw new Error('Unexpected order payload.');
        }
        const detail = payload as OrderDetailRecord;
        setOrderDetail(detail);
      } catch (err) {
        console.error('Order lookup failed', err);
        setOrderError(err instanceof Error ? err.message : 'Failed to load order details.');
      } finally {
        setOrderLoading(false);
      }
    },
    [ensureTenant, buildHeaders]
  );

  useEffect(() => {
    const params = new URLSearchParams(location.search);
    const orderIdParam = params.get('orderId');
    if (orderIdParam) {
      setLookupOrderId(orderIdParam);
      void loadOrderDetail(orderIdParam);
    }
  }, [location.search, loadOrderDetail]);

  const fetchReturns = useCallback(async () => {
    if (!ensureTenant()) return;
    setHistoryLoading(true);
    setHistoryError(null);

    try {
      const params = new URLSearchParams();
      params.set('limit', String(PAGE_SIZE));
      params.set('offset', String(historyPage * PAGE_SIZE));
      if (historyFilters.orderId.trim()) params.set('order_id', historyFilters.orderId.trim());
      if (historyFilters.storeId.trim()) params.set('store_id', historyFilters.storeId.trim());
      if (historyFilters.startDate) params.set('start_date', historyFilters.startDate);
      if (historyFilters.endDate) params.set('end_date', historyFilters.endDate);

      const response = await fetch(`${ORDER_SERVICE_URL}/returns?${params.toString()}`, {
        method: 'GET',
        headers: buildHeaders(),
      });
      if (!response.ok) {
        throw new Error(`Failed to load returns (${response.status})`);
      }
      const payload = (await response.json()) as unknown;
      const records: ReturnSummaryRecord[] = Array.isArray(payload)
        ? payload
            .filter((value: unknown): value is ReturnSummaryRecord => {
              if (typeof value !== 'object' || value === null) return false;
              const candidate = value as Record<string, unknown>;
              return (
                typeof candidate.id === 'string' &&
                typeof candidate.order_id === 'string' &&
                typeof candidate.total === 'number' &&
                typeof candidate.created_at === 'string'
              );
            })
            .map((value) => value)
        : [];
      setReturns(records);
      setHistoryHasNext(records.length === PAGE_SIZE);
    } catch (err) {
      console.error('Failed to load returns', err);
      setReturns([]);
      setHistoryHasNext(false);
      setHistoryError(err instanceof Error ? err.message : 'Unable to load returns.');
    } finally {
      setHistoryLoading(false);
    }
  }, [ensureTenant, historyFilters, historyPage, buildHeaders]);

  useEffect(() => {
    void fetchReturns();
  }, [fetchReturns]);

  const handleQuantityChange = (productId: string, maxQuantity: number, value: number) => {
    const safeValue = Number.isFinite(value) ? Math.max(0, Math.min(maxQuantity, Math.floor(value))) : 0;
    setReturnQuantities((prev) => ({ ...prev, [productId]: safeValue }));
  };

  const remainingQuantity = (item: OrderLineItemRecord): number => {
    return Math.max(0, item.quantity - item.returned_quantity);
  };

  const submitReturn = async () => {
    if (!orderDetail) return;
    if (!ensureTenant()) return;

    const selectedItems = orderDetail.items
      .map((item) => ({
        product_id: item.product_id,
        quantity: returnQuantities[item.product_id] ?? 0,
        unit_price: item.unit_price,
      }))
      .filter((entry) => entry.quantity > 0);

    if (selectedItems.length === 0) {
      setSubmitError('Select at least one item to return.');
      return;
    }

    const total = selectedItems.reduce((sum, entry) => sum + entry.unit_price * entry.quantity, 0);

    setSubmitLoading(true);
    setSubmitError(null);
    setSubmitSuccess(null);

    try {
      const payload = {
        order_id: orderDetail.order.id,
        items: selectedItems.map(({ product_id, quantity }) => ({ product_id, quantity })),
        total: Number(total.toFixed(2)),
        reason: returnReason.trim() || undefined,
      };

      const response = await fetch(`${ORDER_SERVICE_URL}/orders/refund`, {
        method: 'POST',
        headers: buildHeaders(true),
        body: JSON.stringify(payload),
      });

      if (!response.ok) {
        const message = await response.text();
        throw new Error(message || `Refund failed (${response.status})`);
      }

      setSubmitSuccess('Return processed successfully.');
      setReturnQuantities({});
      setReturnReason('');
      await fetchReturns();
      await loadOrderDetail(orderDetail.order.id);
    } catch (err) {
      console.error('Return submission failed', err);
      setSubmitError(err instanceof Error ? err.message : 'Return submission failed.');
    } finally {
      setSubmitLoading(false);
    }
  };

  const resetFilters = () => {
    setHistoryFilters(defaultFilters);
    setHistoryPage(0);
  };

  const handleHistoryFilterChange = (field: keyof ReturnFilters, value: string) => {
    setHistoryFilters((prev) => ({ ...prev, [field]: value }));
    setHistoryPage(0);
  };

  const handlePrevHistory = () => {
    setHistoryPage((prev) => Math.max(prev - 1, 0));
  };

  const handleNextHistory = () => {
    if (historyHasNext) setHistoryPage((prev) => prev + 1);
  };

  return (
    <div className="min-h-screen bg-gray-100 px-6 py-8">
      <div className="mx-auto max-w-6xl space-y-8">
        <header className="rounded-lg bg-white p-6 shadow">
          <h1 className="text-2xl font-semibold text-gray-800">Returns &amp; Refunds</h1>
          <p className="text-sm text-gray-600">Review prior returns and process new refunds against completed orders.</p>
        </header>

        <section className="rounded-lg bg-white p-6 shadow">
          <h2 className="mb-4 text-lg font-semibold text-gray-800">Initiate a Return</h2>
          <div className="grid gap-4 md:grid-cols-3">
            <label className="flex flex-col text-sm font-medium text-gray-700">
              Order ID
              <div className="mt-1 flex gap-2">
                <input
                  className="flex-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900"
                  value={lookupOrderId}
                  onChange={(event) => setLookupOrderId(event.target.value)}
                  placeholder="Enter order ID"
                />
                <button
                  type="button"
                  className="rounded bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700"
                  onClick={() => lookupOrderId.trim() && void loadOrderDetail(lookupOrderId.trim())}
                  disabled={orderLoading}
                >
                  Lookup
                </button>
              </div>
            </label>
            <label className="flex flex-col text-sm font-medium text-gray-700">
              Reason (optional)
              <input
                className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900"
                value={returnReason}
                onChange={(event) => setReturnReason(event.target.value)}
                placeholder="Reason for return"
              />
            </label>
          </div>
          {orderLoading && <p className="mt-4 text-sm text-gray-600">Loading order details...</p>}
          {orderError && <p className="mt-4 text-sm text-red-600">{orderError}</p>}
          {orderDetail && (
            <div className="mt-4 space-y-4">
              <div className="rounded border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700">
                <p><strong>Order:</strong> {orderDetail.order.id}</p>
                <p><strong>Status:</strong> {orderDetail.order.status}</p>
                <p><strong>Payment:</strong> {orderDetail.order.payment_method}</p>
                <p><strong>Total:</strong> ${orderDetail.order.total.toFixed(2)}</p>
              </div>
              <div className="overflow-x-auto">
                <table className="min-w-full divide-y divide-gray-200 text-sm">
                  <thead className="bg-gray-100">
                    <tr>
                      <th className="px-4 py-2 text-left font-medium text-gray-600">Item</th>
                      <th className="px-4 py-2 text-right font-medium text-gray-600">Sold</th>
                      <th className="px-4 py-2 text-right font-medium text-gray-600">Already Returned</th>
                      <th className="px-4 py-2 text-right font-medium text-gray-600">Unit Price</th>
                      <th className="px-4 py-2 text-right font-medium text-gray-600">Return Qty</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-gray-200">
                    {orderDetail.items.map((item) => {
                      const remaining = remainingQuantity(item);
                      return (
                        <tr key={item.product_id}>
                          <td className="px-4 py-2 text-gray-700">{item.product_name || item.product_id}</td>
                          <td className="px-4 py-2 text-right text-gray-700">{item.quantity}</td>
                          <td className="px-4 py-2 text-right text-gray-700">{item.returned_quantity}</td>
                          <td className="px-4 py-2 text-right text-gray-700">${item.unit_price.toFixed(2)}</td>
                          <td className="px-4 py-2 text-right text-gray-700">
                            <input
                              type="number"
                              min={0}
                              max={remaining}
                              value={returnQuantities[item.product_id] ?? 0}
                              onChange={(event) => handleQuantityChange(item.product_id, remaining, Number(event.target.value))}
                              className="w-24 rounded border border-gray-300 px-2 py-1 text-right"
                              disabled={remaining === 0 || submitLoading}
                            />
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
              {submitError && <p className="text-sm text-red-600">{submitError}</p>}
              {submitSuccess && <p className="text-sm text-emerald-600">{submitSuccess}</p>}
              <div className="flex justify-end">
                <button
                  type="button"
                  className="rounded bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-700 disabled:cursor-not-allowed disabled:bg-emerald-400"
                  onClick={() => void submitReturn()}
                  disabled={submitLoading}
                >
                  {submitLoading ? 'Processing...' : 'Process Return'}
                </button>
              </div>
            </div>
          )}
        </section>

        <section className="rounded-lg bg-white p-6 shadow">
          <div className="mb-4 flex items-center justify-between">
            <h2 className="text-lg font-semibold text-gray-800">Return History</h2>
            <div className="space-x-2 text-sm">
              <button
                type="button"
                className="rounded bg-gray-200 px-3 py-1 font-medium text-gray-700 hover:bg-gray-300"
                onClick={resetFilters}
              >
                Reset Filters
              </button>
              <button
                type="button"
                className="rounded bg-blue-600 px-3 py-1 font-medium text-white hover:bg-blue-700"
                onClick={() => void fetchReturns()}
                disabled={historyLoading}
              >
                Refresh
              </button>
            </div>
          </div>
          <div className="mb-4 grid gap-4 md:grid-cols-4">
            <label className="flex flex-col text-sm font-medium text-gray-700">
              Order ID
              <input
                className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900"
                value={historyFilters.orderId}
                onChange={(event) => handleHistoryFilterChange('orderId', event.target.value)}
                placeholder="Filter by order"
              />
            </label>
            <label className="flex flex-col text-sm font-medium text-gray-700">
              Store ID
              <input
                className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900"
                value={historyFilters.storeId}
                onChange={(event) => handleHistoryFilterChange('storeId', event.target.value)}
                placeholder="Filter by store"
              />
            </label>
            <label className="flex flex-col text-sm font-medium text-gray-700">
              Start Date
              <input
                type="date"
                className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900"
                value={historyFilters.startDate}
                onChange={(event) => handleHistoryFilterChange('startDate', event.target.value)}
              />
            </label>
            <label className="flex flex-col text-sm font-medium text-gray-700">
              End Date
              <input
                type="date"
                className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900"
                value={historyFilters.endDate}
                onChange={(event) => handleHistoryFilterChange('endDate', event.target.value)}
              />
            </label>
          </div>

          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-gray-200 text-sm">
              <thead className="bg-gray-100">
                <tr>
                  <th className="px-4 py-2 text-left font-medium text-gray-600">Return ID</th>
                  <th className="px-4 py-2 text-left font-medium text-gray-600">Order ID</th>
                  <th className="px-4 py-2 text-left font-medium text-gray-600">Store</th>
                  <th className="px-4 py-2 text-left font-medium text-gray-600">Reason</th>
                  <th className="px-4 py-2 text-right font-medium text-gray-600">Total</th>
                  <th className="px-4 py-2 text-left font-medium text-gray-600">Created</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-200">
                {historyLoading ? (
                  <tr>
                    <td colSpan={6} className="px-4 py-6 text-center text-gray-600">
                      Loading return history...
                    </td>
                  </tr>
                ) : returns.length === 0 ? (
                  <tr>
                    <td colSpan={6} className="px-4 py-6 text-center text-gray-600">
                      {historyError ? historyError : 'No returns found for the selected filters.'}
                    </td>
                  </tr>
                ) : (
                  returns.map((entry) => (
                    <tr key={entry.id}>
                      <td className="px-4 py-2 font-mono text-xs text-blue-600">{entry.id}</td>
                      <td className="px-4 py-2 font-mono text-xs text-gray-800">{entry.order_id}</td>
                      <td className="px-4 py-2 text-gray-700">{entry.store_id ?? '--'}</td>
                      <td className="px-4 py-2 text-gray-700">{entry.reason ?? '--'}</td>
                      <td className="px-4 py-2 text-right text-gray-700">${entry.total.toFixed(2)}</td>
                      <td className="px-4 py-2 text-gray-700">{new Date(entry.created_at).toLocaleString()}</td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </div>
          <div className="mt-3 flex items-center justify-between border-t border-gray-200 pt-3 text-sm text-gray-600">
            <div>Page {historyPage + 1}</div>
            <div className="space-x-2">
              <button
                type="button"
                className="rounded px-3 py-1 font-medium text-gray-700 hover:bg-gray-200 disabled:cursor-not-allowed disabled:text-gray-400"
                onClick={handlePrevHistory}
                disabled={historyPage === 0 || historyLoading}
              >
                Previous
              </button>
              <button
                type="button"
                className="rounded px-3 py-1 font-medium text-gray-700 hover:bg-gray-200 disabled:cursor-not-allowed disabled:text-gray-400"
                onClick={handleNextHistory}
                disabled={!historyHasNext || historyLoading}
              >
                Next
              </button>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
};

export default ReturnsPage;

