import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useAuth } from "../AuthContext";
import { withRoleGuard } from "../components/RoleGuard";
import { MANAGER_ROLES } from "../rbac";
import { resolveServiceUrl } from "../utils/env";
import "./AdminSectionModern.css";

const ORDER_SERVICE_URL = resolveServiceUrl(
  "VITE_ORDER_SERVICE_URL",
  "http://localhost:8084",
);
const PAGE_SIZE = 25;

const STATUS_OPTIONS = [
  "PENDING",
  "COMPLETED",
  "PARTIAL_REFUNDED",
  "REFUNDED",
  "VOIDED",
];
const PAYMENT_OPTIONS = ["all", "cash", "card", "crypto"];

interface OrderRecord {
  id: string;
  status: string;
  total: number;
  payment_method: string;
  created_at: string;
  customer_name?: string | null;
  customer_email?: string | null;
  store_id?: string | null;
}

interface FiltersState {
  orderId: string;
  status: string;
  paymentMethod: string;
  customerTerm: string;
  startDate: string;
  endDate: string;
  storeId: string;
}

const defaultFilters: FiltersState = {
  orderId: "",
  status: "all",
  paymentMethod: "all",
  customerTerm: "",
  startDate: "",
  endDate: "",
  storeId: "",
};

const OrdersPageContent: React.FC = () => {
  const { currentUser, token, isLoggedIn } = useAuth();
  const navigate = useNavigate();

  const [filters, setFilters] = useState<FiltersState>(defaultFilters);
  const [page, setPage] = useState(0);
  const [orders, setOrders] = useState<OrderRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hasNextPage, setHasNextPage] = useState(false);

  const [receiptOrderId, setReceiptOrderId] = useState<string | null>(null);
  const [receiptLoading, setReceiptLoading] = useState(false);
  const [receiptError, setReceiptError] = useState<string | null>(null);
  const [receiptBody, setReceiptBody] = useState("");

  const tenantId = useMemo(() => {
    return currentUser?.tenant_id ? String(currentUser.tenant_id) : null;
  }, [currentUser]);

  useEffect(() => { if (!isLoggedIn) void navigate("/login", { replace: true }); }, [isLoggedIn, navigate]);

  const ensureTenant = useCallback(() => {
    if (!tenantId) { setError("Unable to determine tenant context."); return false; }
    return true;
  }, [tenantId]);

  const buildHeaders = useCallback((acceptMarkdown = false): Record<string, string> => {
    const headers: Record<string, string> = {};
    if (tenantId) headers["X-Tenant-ID"] = tenantId;
    if (token) headers["Authorization"] = `Bearer ${token}`;
    headers["Accept"] = acceptMarkdown ? "text/markdown" : "application/json";
    return headers;
  }, [tenantId, token]);

  const fetchOrders = useCallback(async () => {
    if (!ensureTenant()) return;
    setLoading(true); setError(null);
    try {
      const params = new URLSearchParams();
      params.set("limit", String(PAGE_SIZE));
      params.set("offset", String(page * PAGE_SIZE));
      if (filters.orderId.trim()) params.set("order_id", filters.orderId.trim());
      if (filters.status !== "all") params.set("status", filters.status.trim());
      if (filters.paymentMethod !== "all") params.set("payment_method", filters.paymentMethod.trim().toLowerCase());
      if (filters.customerTerm.trim()) params.set("customer", filters.customerTerm.trim());
      if (filters.startDate) params.set("start_date", filters.startDate);
      if (filters.endDate) params.set("end_date", filters.endDate);
      if (filters.storeId.trim()) params.set("store_id", filters.storeId.trim());
      const response = await fetch(`${ORDER_SERVICE_URL}/orders?${params.toString()}`, { method: "GET", headers: buildHeaders() });
      if (!response.ok) throw new Error(`Order search failed (${response.status})`);
      const payload = (await response.json()) as unknown;
      const records: OrderRecord[] = Array.isArray(payload) ? payload.filter((value: unknown): value is OrderRecord => {
        if (typeof value !== "object" || value === null) return false;
        const candidate = value as Record<string, unknown>;
        return typeof candidate.id === "string" && typeof candidate.status === "string" && typeof candidate.total === "number" && typeof candidate.payment_method === "string" && typeof candidate.created_at === "string";
      }).map((v) => v) : [];
      setOrders(records); setHasNextPage(records.length === PAGE_SIZE);
    } catch (err) {
      console.error("Failed to fetch orders", err);
      setOrders([]); setHasNextPage(false);
      setError(err instanceof Error ? err.message : "Unable to load orders.");
    } finally { setLoading(false); }
  }, [ensureTenant, filters, page, buildHeaders]);

  useEffect(() => { void fetchOrders(); }, [fetchOrders]);

  const handleFilterChange = (field: keyof FiltersState, value: string) => { setFilters((prev) => ({ ...prev, [field]: value })); setPage(0); };
  const resetFilters = () => { setFilters(defaultFilters); setPage(0); };
  const handlePrevPage = () => setPage((prev) => Math.max(prev - 1, 0));
  const handleNextPage = () => { if (hasNextPage) setPage((prev) => prev + 1); };

  const closeReceipt = () => { setReceiptOrderId(null); setReceiptBody(""); setReceiptError(null); setReceiptLoading(false); };
  const viewReceipt = useCallback(async (orderId: string) => {
    if (!ensureTenant()) return;
    setReceiptOrderId(orderId); setReceiptLoading(true); setReceiptError(null); setReceiptBody("");
    try { const response = await fetch(`${ORDER_SERVICE_URL}/orders/${orderId}/receipt`, { method: "GET", headers: buildHeaders(true) });
      if (!response.ok) throw new Error(`Receipt fetch failed (${response.status})`);
      const text = await response.text(); setReceiptBody(text);
    } catch (err) { console.error("Failed to load receipt", err); setReceiptError(err instanceof Error ? err.message : "Unable to load receipt."); }
    finally { setReceiptLoading(false); }
  }, [ensureTenant, buildHeaders]);

  const navigateToReturns = (orderId: string) => { void navigate(`/returns?orderId=${encodeURIComponent(orderId)}`); };

  const renderReceiptModal = () => {
    if (!receiptOrderId) return null;
    return (
      <div className="fixed inset-0 z-40 flex items-center justify-center bg-black bg-opacity-60 px-4">
        <div className="w-full max-w-2xl rounded-lg bg-white p-6 shadow-xl">
          <div className="mb-4 flex items-center justify-between">
            <h2 className="text-lg font-semibold">Receipt Preview</h2>
            <button type="button" className="rounded bg-gray-200 px-3 py-1 text-sm font-medium text-gray-700 hover:bg-gray-300" onClick={closeReceipt}>Close</button>
          </div>
          {receiptLoading ? (<p className="text-sm text-gray-600">Loading receipt...</p>) : receiptError ? (<p className="text-sm text-red-600">{receiptError}</p>) : (
            <pre className="max-h-96 overflow-y-auto whitespace-pre-wrap rounded border border-gray-200 bg-gray-50 p-3 text-sm text-gray-800">{receiptBody || "No receipt content available."}</pre>
          )}
        </div>
      </div>
    );
  };

  return (
    <div className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col" style={{ fontFamily: "Raleway, sans-serif", background: "linear-gradient(135deg, #f8fafc 0%, #e6f7fa 100%)" }}>
      <div className="admin-section-modern">
        <div className="admin-section-header">
          <div>
            <h2>Orders</h2>
            <p>Search and inspect orders, receipts, and returns.</p>
          </div>
          <div className="flex items-center gap-2">
            <button type="button" className="admin-section-btn" onClick={() => void fetchOrders()} disabled={loading}>{loading ? "Loading…" : "Refresh"}</button>
            <button type="button" className="admin-section-btn" onClick={resetFilters} disabled={loading}>Reset Filters</button>
            <button className="admin-section-btn" onClick={() => void navigate("/home")} type="button">Back to Admin Home</button>
          </div>
        </div>

        <div className="admin-section-content">
          {error && (<div className="mb-4 rounded-md bg-red-100 px-4 py-3 text-sm text-red-700 dark:bg-red-900/40 dark:text-red-200">{error}</div>)}

          <div className="mb-6 grid gap-4 rounded-lg bg-white p-4 shadow">
            <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
              <label className="flex flex-col text-sm font-medium text-gray-700">Order ID
                <input className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900" value={filters.orderId} onChange={(e) => handleFilterChange("orderId", e.target.value)} placeholder="Search by order ID" />
              </label>
              <label className="flex flex-col text-sm font-medium text-gray-700">Customer (name or email)
                <input className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900" value={filters.customerTerm} onChange={(e) => handleFilterChange("customerTerm", e.target.value)} placeholder="Search customer" />
              </label>
              <label className="flex flex-col text-sm font-medium text-gray-700">Store ID
                <input className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900" value={filters.storeId} onChange={(e) => handleFilterChange("storeId", e.target.value)} placeholder="Filter by store" />
              </label>
            </div>
            <div className="grid grid-cols-1 gap-4 md:grid-cols-4">
              <label className="flex flex-col text-sm font-medium text-gray-700">Status
                <select className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900" value={filters.status} onChange={(e) => handleFilterChange("status", e.target.value)}>
                  <option value="all">All</option>
                  {STATUS_OPTIONS.map((status) => (<option key={status} value={status}>{status}</option>))}
                </select>
              </label>
              <label className="flex flex-col text-sm font-medium text-gray-700">Payment Method
                <select className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900" value={filters.paymentMethod} onChange={(e) => handleFilterChange("paymentMethod", e.target.value)}>
                  {PAYMENT_OPTIONS.map((method) => (<option key={method} value={method}>{method === "all" ? "All" : method.charAt(0).toUpperCase() + method.slice(1)}</option>))}
                </select>
              </label>
              <label className="flex flex-col text-sm font-medium text-gray-700">Start Date
                <input type="date" className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900" value={filters.startDate} onChange={(e) => handleFilterChange("startDate", e.target.value)} />
              </label>
              <label className="flex flex-col text-sm font-medium text-gray-700">End Date
                <input type="date" className="mt-1 rounded border border-gray-300 px-3 py-2 text-sm text-gray-900" value={filters.endDate} onChange={(e) => handleFilterChange("endDate", e.target.value)} />
              </label>
            </div>
          </div>

          <div className="rounded-lg bg-white dark:bg-gray-800 shadow">
            <div className="overflow-x-auto">
              <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700 text-sm">
                <thead className="bg-gray-50 dark:bg-gray-700">
                  <tr>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-300">Order ID</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-300">Created</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-300">Status</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-300">Payment</th>
                    <th className="px-4 py-3 text-right font-medium text-gray-600 dark:text-gray-300">Total</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-300">Customer</th>
                    <th className="px-4 py-3 text-left font-medium text-gray-600 dark:text-gray-300">Store</th>
                    <th className="px-4 py-3 text-right font-medium text-gray-600 dark:text-gray-300">Actions</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-gray-700 bg-white dark:bg-gray-800">
                  {loading ? (
                    <tr><td colSpan={8} className="px-4 py-8 text-center text-sm text-gray-600 dark:text-gray-300">Loading orders...</td></tr>
                  ) : orders.length === 0 ? (
                    <tr><td colSpan={8} className="px-4 py-8 text-center text-sm text-gray-600 dark:text-gray-300">{error ? error : "No orders found for the selected filters."}</td></tr>
                  ) : (
                    orders.map((order) => {
                      const created = new Date(order.created_at);
                      const customerLabel = order.customer_name ? (order.customer_email ? `${order.customer_name} (${order.customer_email})` : order.customer_name) : (order.customer_email ?? "--");
                      return (
                        <tr key={order.id} className="hover:bg-gray-50 dark:hover:bg-gray-700/40">
                          <td className="px-4 py-3 font-mono text-xs text-blue-600 dark:text-blue-400">{order.id}</td>
                          <td className="px-4 py-3 text-gray-700 dark:text-gray-200">{created.toLocaleString()}</td>
                          <td className="px-4 py-3 text-gray-700 dark:text-gray-200">{order.status}</td>
                          <td className="px-4 py-3 text-gray-700 dark:text-gray-200">{order.payment_method}</td>
                          <td className="px-4 py-3 text-right text-gray-700 dark:text-gray-200">${order.total.toFixed(2)}</td>
                          <td className="px-4 py-3 text-gray-700 dark:text-gray-200">{customerLabel}</td>
                          <td className="px-4 py-3 text-gray-700 dark:text-gray-200">{order.store_id ?? "--"}</td>
                          <td className="px-4 py-3 text-right text-sm">
                            <div className="flex flex-wrap justify-end gap-2">
                              <button type="button" className="text-sm text-indigo-600 hover:underline dark:text-indigo-400" onClick={() => void viewReceipt(order.id)}>View Receipt</button>
                              <button type="button" className="text-sm text-indigo-600 hover:underline dark:text-indigo-400" onClick={() => navigateToReturns(order.id)}>Start Return</button>
                            </div>
                          </td>
                        </tr>
                      );
                    })
                  )}
                </tbody>
              </table>
            </div>
            <div className="flex items-center justify-between border-t border-gray-200 px-4 py-3 text-sm text-gray-600 dark:text-gray-300">
              <div>Page {page + 1}</div>
              <div className="space-x-2">
                <button type="button" className="rounded px-3 py-1 font-medium text-gray-700 hover:bg-gray-200 disabled:cursor-not-allowed disabled:text-gray-400" onClick={handlePrevPage} disabled={page === 0 || loading}>Previous</button>
                <button type="button" className="rounded px-3 py-1 font-medium text-gray-700 hover:bg-gray-200 disabled:cursor-not-allowed disabled:text-gray-400" onClick={handleNextPage} disabled={!hasNextPage || loading}>Next</button>
              </div>
            </div>
          </div>

          <div style={{ textAlign: "right", marginTop: "2rem" }}>
            <button className="admin-section-btn" onClick={() => void navigate("/home")} type="button">Back to Admin Home</button>
          </div>
        </div>
      </div>
      {renderReceiptModal()}
    </div>
  );
};

const OrdersPage = withRoleGuard(OrdersPageContent, MANAGER_ROLES, { message: "Manager or administrator role required to review orders." });
export default OrdersPage;
