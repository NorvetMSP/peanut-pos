import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useAuth } from "../AuthContext";
import { withRoleGuard } from "../components/RoleGuard";
import { MANAGER_ROLES } from "../rbac";
import "./AdminSectionModern.css";

const resolveBaseUrl = (raw: string | undefined, fallback: string): string => {
  const candidate =
    typeof raw === "string" && raw.trim().length > 0 ? raw : fallback;
  return candidate.replace(/\/$/, "");
};

type EnvRecord = Record<string, string | undefined>;
const { VITE_ANALYTICS_SERVICE_URL, VITE_PRODUCT_SERVICE_URL } = (import.meta
  .env ?? {}) as EnvRecord;
const ANALYTICS_SERVICE_URL = resolveBaseUrl(
  VITE_ANALYTICS_SERVICE_URL,
  "http://localhost:8082",
);
const PRODUCT_SERVICE_URL = resolveBaseUrl(
  VITE_PRODUCT_SERVICE_URL,
  "http://localhost:8081",
);

type SummaryResponse = {
  today_orders: number;
  today_revenue: number;
  top_items: Array<{ product_id: string; quantity: number }>;
};

type ProductNameMap = Record<string, string>;

type SummaryJson = Record<string, unknown> & {
  today_orders?: unknown;
  today_revenue?: unknown;
  top_items?: unknown;
};

type ProductJson = Record<string, unknown>;

const isSummaryResponse = (value: unknown): value is SummaryResponse => {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as SummaryJson;
  return (
    typeof candidate.today_orders === "number" &&
    typeof candidate.today_revenue === "number" &&
    Array.isArray(candidate.top_items)
  );
};

const DashboardPageContent: React.FC = () => {
  const { isLoggedIn, currentUser, token } = useAuth();
  const navigate = useNavigate();

  const [summary, setSummary] = useState<SummaryResponse | null>(null);
  const [alerts, setAlerts] = useState<string[]>([]);
  const [productNames, setProductNames] = useState<ProductNameMap>({});
  const [isLoading, setIsLoading] = useState(true);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isLoggedIn) void navigate("/login", { replace: true });
  }, [isLoggedIn, navigate]);

  const tenantId = currentUser?.tenant_id
    ? String(currentUser.tenant_id)
    : null;

  const buildHeaders = useCallback((): Record<string, string> => {
    const headers: Record<string, string> = {};
    if (tenantId) headers["X-Tenant-ID"] = tenantId;
    if (token) headers["Authorization"] = `Bearer ${token}`;
    return headers;
  }, [tenantId, token]);

  const ensureTenantContext = useCallback((): boolean => {
    if (!tenantId) {
      setError("Tenant context is unavailable. Please log out and back in.");
      setSummary(null);
      setAlerts([]);
      setProductNames({});
      return false;
    }
    return true;
  }, [tenantId]);

  const fetchSummary = useCallback(
    async (headers: Record<string, string>): Promise<SummaryResponse> => {
      const response = await fetch(
        `${ANALYTICS_SERVICE_URL}/analytics/summary`,
        {
          headers,
        },
      );
      if (!response.ok) {
        throw new Error(`Summary request failed (${response.status})`);
      }
      const payload = (await response.json()) as unknown;
      if (!isSummaryResponse(payload)) {
        throw new Error("Summary response malformed");
      }
      return payload;
    },
    [],
  );

  const fetchAnomalies = useCallback(
    async (headers: Record<string, string>): Promise<string[]> => {
      const response = await fetch(
        `${ANALYTICS_SERVICE_URL}/analytics/anomalies`,
        {
          headers,
        },
      );
      if (!response.ok) {
        throw new Error(`Anomaly request failed (${response.status})`);
      }
      const data = (await response.json()) as unknown;
      if (!Array.isArray(data)) return [];
      return data.filter((entry): entry is string => typeof entry === "string");
    },
    [],
  );

  const fetchProducts = useCallback(
    async (headers: Record<string, string>): Promise<ProductNameMap> => {
      const response = await fetch(`${PRODUCT_SERVICE_URL}/products`, {
        headers,
      });
      if (!response.ok) {
        throw new Error(`Products request failed (${response.status})`);
      }
      const data = (await response.json()) as unknown;
      if (!Array.isArray(data)) return {};
      const map: ProductNameMap = {};
      for (const entry of data) {
        if (typeof entry !== "object" || entry === null) continue;
        const candidate = entry as ProductJson;
        const id = typeof candidate.id === "string" ? candidate.id : null;
        const name = typeof candidate.name === "string" ? candidate.name : null;
        if (id && name) map[id] = name;
      }
      return map;
    },
    [],
  );

  const loadData = useCallback(
    async (opts?: { initial?: boolean }) => {
      const isInitial = opts?.initial ?? false;
      if (!ensureTenantContext()) {
        if (isInitial) setIsLoading(false);
        return;
      }

      setError(null);
      if (isInitial) {
        setIsLoading(true);
      } else {
        setIsRefreshing(true);
      }

      const headers = buildHeaders();
      try {
        const [summaryResult, anomaliesResult, productsResult] =
          await Promise.allSettled([
            fetchSummary(headers),
            fetchAnomalies(headers),
            fetchProducts(headers),
          ]);

        if (summaryResult.status === "fulfilled") {
          setSummary(summaryResult.value);
        } else {
          console.error("Failed to load summary", summaryResult.reason);
          setSummary(null);
          setError("Unable to load dashboard summary. Please try again.");
        }

        if (anomaliesResult.status === "fulfilled") {
          setAlerts(anomaliesResult.value);
        } else {
          console.warn("Failed to load anomalies", anomaliesResult.reason);
          setAlerts([]);
        }

        if (productsResult.status === "fulfilled") {
          setProductNames(productsResult.value);
        } else {
          console.warn("Failed to load product names", productsResult.reason);
        }
      } catch (err) {
        console.error("Dashboard load failure", err);
        setError("Unexpected error loading dashboard.");
      } finally {
        if (isInitial) {
          setIsLoading(false);
        } else {
          setIsRefreshing(false);
        }
      }
    },
    [
      buildHeaders,
      ensureTenantContext,
      fetchAnomalies,
      fetchProducts,
      fetchSummary,
    ],
  );

  useEffect(() => {
    void loadData({ initial: true });
  }, [loadData]);

  const topItems = useMemo(() => summary?.top_items ?? [], [summary]);
  const ordersToday = summary?.today_orders ?? 0;
  const revenueToday = summary?.today_revenue ?? 0;
  const showDemoBanner = summary != null && ordersToday === 0;

  return (
    <div className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col">
      <div className="admin-section-modern">
        <div className="admin-section-header">
          <div>
            <h2>Dashboard</h2>
            <p>Today's performance and key alerts.</p>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              className="admin-section-btn"
              onClick={() => void loadData()}
              disabled={isLoading || isRefreshing}
            >
              {isRefreshing ? "Refreshing..." : "Refresh"}
            </button>
            <button
              className="admin-section-btn"
              onClick={() => void navigate("/home")}
              type="button"
            >
              Back to Admin Home
            </button>
          </div>
        </div>

        <div className="admin-section-content">
          {error && (
            <div className="mb-4 rounded-md bg-red-100 px-4 py-3 text-sm text-red-700 dark:bg-red-900/40 dark:text-red-200">
              {error}
            </div>
          )}

          {isLoading ? (
            <div className="text-center text-gray-600 dark:text-gray-300 py-8">
              Loading dashboard...
            </div>
          ) : (
            <div className="grid gap-6">
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <div className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow">
                  <h3 className="text-sm font-semibold text-gray-500 dark:text-gray-300">
                    Today's Orders
                  </h3>
                  <p className="mt-2 text-3xl font-bold text-gray-900 dark:text-gray-100">
                    {ordersToday}
                  </p>
                </div>
                <div className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow">
                  <h3 className="text-sm font-semibold text-gray-500 dark:text-gray-300">
                    Today's Revenue
                  </h3>
                  <p className="mt-2 text-3xl font-bold text-gray-900 dark:text-gray-100">
                    ${revenueToday.toFixed(2)}
                  </p>
                </div>
              </div>

              {showDemoBanner && (
                <div className="rounded-lg border border-dashed border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 p-4 text-sm text-gray-600 dark:text-gray-300">
                  Demo mode - no sales recorded today yet.
                </div>
              )}

              <div className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow">
                <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
                  Top 5 Products
                </h3>
                {topItems.length === 0 ? (
                  <p className="mt-3 text-sm text-gray-500 dark:text-gray-300">
                    No sales yet today.
                  </p>
                ) : (
                  <ul className="mt-3 space-y-2">
                    {topItems.map((item) => {
                      const label =
                        productNames[item.product_id] ?? item.product_id;
                      return (
                        <li
                          key={item.product_id}
                          className="flex items-center justify-between rounded border border-gray-200 dark:border-gray-700 px-4 py-2 text-sm text-gray-700 dark:text-gray-200"
                        >
                          <span className="font-medium">{label}</span>
                          <span className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-300">
                            {item.quantity} sold
                          </span>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </div>

              {alerts.length > 0 && (
                <div className="rounded-lg bg-amber-100 dark:bg-amber-900/40 p-4 text-amber-900 dark:text-amber-100 shadow">
                  <h3 className="text-lg font-semibold mb-2">Alerts</h3>
                  <ul className="space-y-1 text-sm">
                    {alerts.map((alert) => (
                      <li key={alert}>{alert}</li>
                    ))}
                  </ul>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

const DashboardPage = withRoleGuard(DashboardPageContent, MANAGER_ROLES, {
  message: "Manager or administrator role required to view analytics.",
});

export default DashboardPage;
