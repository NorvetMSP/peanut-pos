import React, { useEffect, useMemo, useState } from 'react';
import { useAuth } from '../AuthContext';
import { useNavigate, useSearchParams } from 'react-router-dom';

const ORDER_SERVICE_URL = (import.meta.env.VITE_ORDER_SERVICE_URL ?? 'http://localhost:8084').replace(/\/$/, '');

type ReturnItem = { product_id: string; qty: number };

type NewItem = { sku: string; qty: number };

export default function ExchangePage() {
  const { token, currentUser, isLoggedIn } = useAuth();
  const [search] = useSearchParams();
  const [originalOrderId, setOriginalOrderId] = useState<string>('');
  const [returnItems, setReturnItems] = useState<ReturnItem[]>([]);
  const [newItems, setNewItems] = useState<NewItem[]>([]);
  const [paymentMethod, setPaymentMethod] = useState<'card'|'cash'>('card');
  const [amountCents, setAmountCents] = useState<number>(0);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<any | null>(null);
  const [loadingOrder, setLoadingOrder] = useState(false);
  const [orderFetchError, setOrderFetchError] = useState<string | null>(null);
  const [orderItems, setOrderItems] = useState<Array<{ product_id: string; name?: string; maxQty: number; selectedQty: number }>>([]);
  const navigate = useNavigate();

  useEffect(() => {
    const pre = search.get('order');
    if (pre) setOriginalOrderId(pre);
  }, [search]);

  // Fetch original order details when ID changes
  useEffect(() => {
    const load = async () => {
      if (!originalOrderId || !token || !currentUser?.tenant_id) {
        setOrderItems([]);
        return;
      }
      setLoadingOrder(true); setOrderFetchError(null);
      try {
        const resp = await fetch(`${ORDER_SERVICE_URL}/orders/${originalOrderId}`, {
          headers: {
            'Accept': 'application/json',
            'Authorization': `Bearer ${token}`,
            'X-Tenant-ID': String(currentUser.tenant_id ?? ''),
            'X-Roles': 'manager',
          },
        });
        if (!resp.ok) throw new Error(`Failed to load order (${resp.status})`);
        const data = await resp.json();
        const items: Array<{ product_id?: unknown; quantity?: unknown; returned_quantity?: unknown; name?: unknown }> = Array.isArray(data?.items) ? data.items : [];
        const mapped = items
          .map((it: { product_id?: unknown; quantity?: unknown; returned_quantity?: unknown; name?: unknown }) => {
            const productId = typeof it.product_id === 'string' ? it.product_id : String(it.product_id ?? '');
            const qty = Number(it.quantity);
            const returned = Number(it.returned_quantity);
            const maxQty = Math.max(0, (Number.isFinite(qty) ? qty : 0) - (Number.isFinite(returned) ? returned : 0));
            const name = typeof it.name === 'string' ? it.name : undefined;
            if (!productId || maxQty <= 0) return null;
            return { product_id: productId, name, maxQty, selectedQty: 0 };
          })
          .filter((v) => Boolean(v)) as Array<{ product_id: string; name?: string; maxQty: number; selectedQty: number }>;
        setOrderItems(mapped);
      } catch (err) {
        setOrderFetchError(err instanceof Error ? err.message : 'Failed to load order');
        setOrderItems([]);
      } finally {
        setLoadingOrder(false);
      }
    };
    void load();
  }, [originalOrderId, token, currentUser?.tenant_id]);

  const canSubmit = useMemo(() => {
    return isLoggedIn && originalOrderId && (returnItems.length > 0 || newItems.length > 0) && !submitting;
  }, [isLoggedIn, originalOrderId, returnItems, newItems, submitting]);

  const addReturn = () => setReturnItems((prev: ReturnItem[]) => [...prev, { product_id: '', qty: 1 }]);
  const addNew = () => setNewItems((prev: NewItem[]) => [...prev, { sku: '', qty: 1 }]);

  const addSelectedReturns = () => {
    const selected = orderItems
      .filter((oi: { product_id: string; name?: string; maxQty: number; selectedQty: number }) => oi.selectedQty > 0)
      .map((oi: { product_id: string; name?: string; maxQty: number; selectedQty: number }) => ({ product_id: oi.product_id, qty: oi.selectedQty }));
    if (selected.length === 0) return;
    setReturnItems((prev: ReturnItem[]) => {
      const combined = [...prev];
      for (const s of selected) {
        // merge by product_id if already present
        const idx = combined.findIndex(r => r.product_id === s.product_id);
        if (idx >= 0) {
          combined[idx] = { ...combined[idx], qty: combined[idx].qty + s.qty };
        } else {
          combined.push({ product_id: s.product_id, qty: s.qty });
        }
      }
      return combined;
    });
  };

  const submit = async () => {
    if (!canSubmit || !token || !currentUser?.tenant_id) return;
    setSubmitting(true); setError(null); setResult(null);
    try {
      const body = {
  return_items: returnItems.filter((r: ReturnItem) => r.product_id && r.qty > 0),
  new_items: newItems.filter((n: NewItem) => n.sku && n.qty > 0),
        payment: { method: paymentMethod, amount_cents: amountCents },
      };
      const resp = await fetch(`${ORDER_SERVICE_URL}/orders/${originalOrderId}/exchange`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${token}`,
          'X-Tenant-ID': String(currentUser.tenant_id ?? ''),
          'X-Roles': 'manager',
        },
        body: JSON.stringify(body),
      });
      if (!resp.ok) {
        const text = await resp.text();
        throw new Error(text || `Exchange failed with ${resp.status}`);
      }
      const data = await resp.json();
      setResult(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="p-4 max-w-3xl mx-auto">
      <h1 className="text-xl font-semibold mb-2">Exchange</h1>
      <div className="space-y-3">
        <label className="block">
          <span className="text-sm">Original Order ID</span>
          <input className="border rounded p-2 w-full" value={originalOrderId} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setOriginalOrderId(e.target.value)} placeholder="UUID" />
        </label>

        {originalOrderId && (
          <div className="border rounded p-3">
            <div className="flex items-center justify-between mb-2">
              <span className="font-medium">Return from original order</span>
              <button className="text-blue-600 disabled:opacity-50" disabled={orderItems.every((i: { selectedQty: number }) => i.selectedQty === 0)} onClick={addSelectedReturns}>Add selected</button>
            </div>
            {loadingOrder && <div>Loading order…</div>}
            {orderFetchError && <div className="text-red-600">{orderFetchError}</div>}
            {!loadingOrder && !orderFetchError && orderItems.length === 0 && (
              <div className="text-gray-600">No eligible items to return.</div>
            )}
            {!loadingOrder && !orderFetchError && orderItems.length > 0 && (
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left"><th className="py-1">Product</th><th className="py-1">Max</th><th className="py-1">Qty</th></tr>
                </thead>
                <tbody>
                  {orderItems.map((it, idx) => (
                    <tr key={it.product_id} className="border-t">
                      <td className="py-1">{it.name ?? it.product_id}</td>
                      <td className="py-1">{it.maxQty}</td>
                      <td className="py-1">
                        <input
                          aria-label={`Return qty ${it.product_id}`}
                          type="number"
                          min={0}
                          max={it.maxQty}
                          className="border rounded p-1 w-24"
                          value={it.selectedQty}
                          onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                            const v = Math.max(0, Math.min(it.maxQty, Number(e.target.value || 0)));
                            setOrderItems(prev => prev.map((x,i) => i===idx ? { ...x, selectedQty: v } : x));
                          }}
                        />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        )}

        <div>
          <div className="flex items-center justify-between mb-1">
            <span className="font-medium">Return Items</span>
            <button className="text-blue-600" onClick={addReturn}>+ Add</button>
          </div>
          {returnItems.map((r, idx) => (
            <div key={idx} className="flex gap-2 mb-2">
              <input className="border rounded p-2 flex-1" placeholder="product_id" value={r.product_id} onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                const v = e.target.value; setReturnItems(prev => prev.map((x,i) => i===idx ? { ...x, product_id: v } : x));
              }} />
              <input type="number" min={1} className="border rounded p-2 w-24" value={r.qty} onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                const v = Math.max(1, Number(e.target.value||1)); setReturnItems(prev => prev.map((x,i) => i===idx ? { ...x, qty: v } : x));
              }} />
            </div>
          ))}
        </div>

        <div>
          <div className="flex items-center justify-between mb-1">
            <span className="font-medium">New Items</span>
            <button className="text-blue-600" onClick={addNew}>+ Add</button>
          </div>
          {newItems.map((n, idx) => (
            <div key={idx} className="flex gap-2 mb-2">
              <input className="border rounded p-2 flex-1" placeholder="sku" value={n.sku} onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                const v = e.target.value; setNewItems(prev => prev.map((x,i) => i===idx ? { ...x, sku: v } : x));
              }} />
              <input type="number" min={1} className="border rounded p-2 w-24" value={n.qty} onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                const v = Math.max(1, Number(e.target.value||1)); setNewItems(prev => prev.map((x,i) => i===idx ? { ...x, qty: v } : x));
              }} />
            </div>
          ))}
        </div>

        <div className="flex gap-4 items-end">
          <label className="block">
            <span className="text-sm">Payment Method</span>
            <select className="border rounded p-2 w-40" value={paymentMethod} onChange={(e: React.ChangeEvent<HTMLSelectElement>) => setPaymentMethod(e.target.value as 'card'|'cash')}>
              <option value="card">Card</option>
              <option value="cash">Cash</option>
            </select>
          </label>
          <label className="block">
            <span className="text-sm">Amount (cents)</span>
            <input type="number" min={0} className="border rounded p-2 w-40" value={amountCents} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setAmountCents(Math.max(0, Number(e.target.value||0)))} />
          </label>
          <button className="bg-blue-600 text-white px-4 py-2 rounded disabled:opacity-50" onClick={submit} disabled={!canSubmit}>{submitting ? 'Submitting…' : 'Submit Exchange'}</button>
        </div>

        {error && <div className="text-red-600">{error}</div>}
        {result && (
          <div className="bg-gray-50 border rounded p-3">
            <div><b>Original:</b> {result.original_order_id}</div>
            <div><b>Exchange:</b> {result.exchange_order_id}</div>
            <div><b>Refunded:</b> {result.refunded_cents}</div>
            <div><b>New Total:</b> {result.new_order_total_cents}</div>
            <div><b>Net:</b> {result.net_delta_cents} ({result.net_direction})</div>
          </div>
        )}

        <div>
          <button className="text-gray-600" onClick={() => navigate('/pos')}>Back to POS</button>
        </div>
      </div>
    </div>
  );
}
