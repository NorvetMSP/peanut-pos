/* eslint-disable react-refresh/only-export-components */
import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { useAuth } from './AuthContext';

const ORDER_SERVICE_URL = (import.meta.env.VITE_ORDER_SERVICE_URL ?? 'http://localhost:8084').replace(/\/$/, '');
const INTEGRATION_GATEWAY_URL = (import.meta.env.VITE_INTEGRATION_GATEWAY_URL ?? 'http://localhost:8083').replace(/\/$/, '');
const ORDER_STATUS_WS_URL = (() => {
  const raw = import.meta.env.VITE_ORDER_STATUS_WS_URL;
  if (typeof raw === 'string' && raw.trim().length > 0) {
    return raw.trim().replace(/\/$/, '');
  }
  try {
    const derived = new URL(ORDER_SERVICE_URL);
    derived.protocol = derived.protocol === 'https:' ? 'wss:' : 'ws:';
    const cleanedPath = derived.pathname.replace(/\/$/, '');
    derived.pathname = `${cleanedPath}/ws/orders`;
    return derived.toString();
  } catch {
    return '';
  }
})();

const OFFLINE_QUEUE_STORAGE_KEY = 'pos-offline-orders';
const RECENT_ORDERS_STORAGE_KEY = 'pos-recent-orders';
const RECENT_ORDERS_LIMIT = 20;
const STATUS_POLL_INTERVAL_MS = (() => {
  const raw = Number(import.meta.env.VITE_ORDER_STATUS_POLL_MS);
  if (Number.isFinite(raw) && raw >= 5000) return raw;
  return 15000;
})();

const AUTO_FLUSH_MIN_INTERVAL_MS = 5000;

const TERMINAL_STATUS_KEYWORDS = ['complete', 'completed', 'accepted', 'fulfilled', 'cancel', 'cancelled', 'void', 'voided', 'declined', 'failed', 'refunded', 'closed'];
const PENDING_STATUS_KEYWORDS = ['pending', 'processing', 'awaiting', 'submitted'];
const PENDING_PAYMENT_KEYWORDS = ['pending', 'processing', 'awaiting', 'requires', 'open'];

export type PaymentMethod = 'card' | 'cash' | 'crypto';

type OrderItemPayload = {
  product_id: string;
  quantity: number;
  unit_price: number;
  line_total: number;
};

export type DraftOrderPayload = {
  items: OrderItemPayload[];
  payment_method: PaymentMethod;
  total: number;
  customer_id?: string;
  metadata?: Record<string, unknown>;
};

type OrderSubmissionPayload = DraftOrderPayload & { offline: boolean };

type OrderResponse = {
  id: string;
  status?: string;
  payment_status?: string;
  paymentUrl?: string | null;
  payment_url?: string | null;
  note?: string;
  [key: string]: unknown;
};

type PaymentResponse = {
  status?: string;
  payment_url?: string;
  paymentUrl?: string;
  [key: string]: unknown;
};

type QueuedOrder = {
  tempId: string;
  payload: OrderSubmissionPayload;
  createdAt: number;
  attempts: number;
  lastError?: string;
};

type OrderHistoryEntry = {
  id: string;
  reference: string;
  status: string;
  paymentStatus?: string;
  paymentMethod: PaymentMethod;
  total: number;
  createdAt: number;
  offline: boolean;
  tempId?: string;
  paymentUrl?: string | null;
  note?: string;
  syncedAt?: number;
};

export type SubmitOrderResult =
  | { status: 'queued'; tempId: string; queuedCount: number }
  | { status: 'submitted'; order: OrderResponse; payment?: PaymentResponse; paymentError?: string };

type SubmitOptions = {
  forceOffline?: boolean;
};

type OrderContextValue = {
  submitOrder: (payload: DraftOrderPayload, options?: SubmitOptions) => Promise<SubmitOrderResult>;
  queuedOrders: QueuedOrder[];
  recentOrders: OrderHistoryEntry[];
  isOnline: boolean;
  isSyncing: boolean;
  retryQueue: () => Promise<void>;
  refreshOrderStatuses: () => Promise<void>;
};

const OrderContext = createContext<OrderContextValue | undefined>(undefined);

const isBrowser = typeof window !== 'undefined';

const normalize = (value?: string | null): string => (value ? value.toLowerCase() : '');
const containsKeyword = (value: string, keywords: string[]) => keywords.some(keyword => value.includes(keyword));
const isTerminalStatus = (status?: string | null): boolean => {
  const normalized = normalize(status);
  if (!normalized) return false;
  return containsKeyword(normalized, TERMINAL_STATUS_KEYWORDS);
};
const isPendingStatus = (status?: string | null): boolean => {
  const normalized = normalize(status);
  if (!normalized) return false;
  return containsKeyword(normalized, PENDING_STATUS_KEYWORDS);
};
const isPendingPaymentStatus = (status?: string | null): boolean => {
  const normalized = normalize(status);
  if (!normalized) return false;
  return containsKeyword(normalized, PENDING_PAYMENT_KEYWORDS);
};

const parseError = (input: unknown): string => {
  if (input instanceof Error) return input.message;
  if (typeof input === 'string') return input;
  try {
    return JSON.stringify(input);
  } catch {
    return 'Unknown error';
  }
};

const readQueueFromStorage = (): QueuedOrder[] => {
  if (!isBrowser) return [];
  const raw = window.localStorage.getItem(OFFLINE_QUEUE_STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const hydrated: QueuedOrder[] = [];
    for (const item of parsed) {
      if (!item || typeof item !== 'object') continue;
      const candidate = item as Record<string, unknown>;
      const tempId = typeof candidate.tempId === 'string' ? candidate.tempId : null;
      const payload = candidate.payload;
      const createdAt = typeof candidate.createdAt === 'number' ? candidate.createdAt : Date.now();
      const attempts = typeof candidate.attempts === 'number' ? candidate.attempts : 0;
      if (!tempId || !payload || typeof payload !== 'object') continue;
      const payloadRecord = payload as Record<string, unknown>;
      const items = Array.isArray(payloadRecord.items) ? payloadRecord.items : [];
      const paymentMethod = payloadRecord.payment_method;
      const total = payloadRecord.total;
      if (items.length === 0 || (paymentMethod !== 'card' && paymentMethod !== 'cash' && paymentMethod !== 'crypto')) {
        continue;
      }
      const normalizedPayload: OrderSubmissionPayload = {
        items: items
          .map(entry => {
            if (!entry || typeof entry !== 'object') return null;
            const row = entry as Record<string, unknown>;
            const productId = row.product_id;
            const quantity = row.quantity;
            const unitPrice = row.unit_price;
            const lineTotal = row.line_total;
            if (typeof productId !== 'string' && typeof productId !== 'number') return null;
            if (!Number.isFinite(Number(quantity)) || !Number.isFinite(Number(unitPrice))) return null;
            const quantityNumber = Number(quantity);
            const priceNumber = Number(unitPrice);
            return {
              product_id: typeof productId === 'string' ? productId : String(productId),
              quantity: quantityNumber,
              unit_price: priceNumber,
              line_total: Number(lineTotal ?? priceNumber * quantityNumber),
            };
          })
          .filter((row): row is OrderItemPayload => Boolean(row)),
        payment_method: paymentMethod as PaymentMethod,
        total: Number(total),
        customer_id: typeof payloadRecord.customer_id === 'string' ? payloadRecord.customer_id : undefined,
        metadata: typeof payloadRecord.metadata === 'object' && payloadRecord.metadata !== null ? (payloadRecord.metadata as Record<string, unknown>) : undefined,
        offline: true,
      };
      if (normalizedPayload.items.length === 0 || !Number.isFinite(normalizedPayload.total)) {
        continue;
      }
      hydrated.push({
        tempId,
        payload: normalizedPayload,
        createdAt,
        attempts,
        lastError: typeof candidate.lastError === 'string' ? candidate.lastError : undefined,
      });
    }
    return hydrated;
  } catch (err) {
    console.warn('Unable to read offline order queue', err);
    return [];
  }
};

const readRecentOrders = (): OrderHistoryEntry[] => {
  if (!isBrowser) return [];
  const raw = window.localStorage.getItem(RECENT_ORDERS_STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .map(item => {
        if (!item || typeof item !== 'object') return null;
        const candidate = item as Record<string, unknown>;
        const reference = typeof candidate.reference === 'string' ? candidate.reference : null;
        const status = typeof candidate.status === 'string' ? candidate.status : null;
        const paymentMethod = candidate.paymentMethod;
        const total = candidate.total;
        if (!reference || !status || (paymentMethod !== 'card' && paymentMethod !== 'cash' && paymentMethod !== 'crypto')) {
          return null;
        }
        return {
          id: typeof candidate.id === 'string' ? candidate.id : reference,
          reference,
          status,
          paymentStatus: typeof candidate.paymentStatus === 'string' ? candidate.paymentStatus : undefined,
          paymentMethod: paymentMethod as PaymentMethod,
          total: Number(total) || 0,
          createdAt: typeof candidate.createdAt === 'number' ? candidate.createdAt : Date.now(),
          offline: Boolean(candidate.offline),
          tempId: typeof candidate.tempId === 'string' ? candidate.tempId : undefined,
          paymentUrl: typeof candidate.paymentUrl === 'string' ? candidate.paymentUrl : undefined,
          note: typeof candidate.note === 'string' ? candidate.note : undefined,
          syncedAt: typeof candidate.syncedAt === 'number' ? candidate.syncedAt : undefined,
        } as OrderHistoryEntry;
      })
      .filter((item): item is OrderHistoryEntry => Boolean(item));
  } catch (err) {
    console.warn('Unable to read recent orders', err);
    return [];
  }
};

const shouldMonitorOrder = (entry: OrderHistoryEntry): boolean => {
  if (entry.offline) return false;
  const normalizedStatus = normalize(entry.status);
  const normalizedPayment = normalize(entry.paymentStatus);
  if (!normalizedStatus && !normalizedPayment) return false;
  if (isPendingStatus(entry.status) || normalizedStatus.includes('submitted')) return true;
  if (!isTerminalStatus(entry.status) && normalizedStatus.includes('queue')) return true;
  if (isPendingPaymentStatus(entry.paymentStatus)) return true;
  if (!normalizedPayment && !isTerminalStatus(entry.status)) return true;
  return false;
};

export const OrderProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const { currentUser, token } = useAuth();
  const tenantId = currentUser?.tenant_id ? String(currentUser.tenant_id) : null;

  const [queuedOrders, setQueuedOrders] = useState<QueuedOrder[]>(() => readQueueFromStorage());
  const [recentOrders, setRecentOrders] = useState<OrderHistoryEntry[]>(() => readRecentOrders());
  const [isOnline, setIsOnline] = useState<boolean>(() => (typeof navigator !== 'undefined' ? navigator.onLine : true));
  const [isSyncing, setIsSyncing] = useState(false);
  const [wsRetryToken, setWsRetryToken] = useState(0);

  const queueRef = useRef<QueuedOrder[]>(queuedOrders);
  const isOnlineRef = useRef<boolean>(isOnline);
  const autoFlushRef = useRef<number>(0);
  const wsRef = useRef<WebSocket | null>(null);
  const wsTimeoutRef = useRef<number | null>(null);

  useEffect(() => {
    queueRef.current = queuedOrders;
  }, [queuedOrders]);

  useEffect(() => {
    isOnlineRef.current = isOnline;
  }, [isOnline]);

  useEffect(() => {
    if (!isBrowser) return;
    window.localStorage.setItem(OFFLINE_QUEUE_STORAGE_KEY, JSON.stringify(queuedOrders));
  }, [queuedOrders]);

  useEffect(() => {
    if (!isBrowser) return;
    window.localStorage.setItem(RECENT_ORDERS_STORAGE_KEY, JSON.stringify(recentOrders.slice(0, RECENT_ORDERS_LIMIT)));
  }, [recentOrders]);

  const buildHeaders = useCallback((): Record<string, string> => {
    if (!tenantId) {
      throw new Error('Tenant context is unavailable. Please log in again.');
    }
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      'X-Tenant-ID': tenantId,
    };
    if (token) headers['Authorization'] = `Bearer ${token}`;
    return headers;
  }, [tenantId, token]);

  const postOrder = useCallback(async (payload: OrderSubmissionPayload): Promise<OrderResponse> => {
    const headers = buildHeaders();
    const response = await fetch(`${ORDER_SERVICE_URL}/orders`, {
      method: 'POST',
      headers,
      body: JSON.stringify(payload),
    });
    if (!response.ok) {
      let message = `Order submission failed (${response.status})`;
      try {
        const text = await response.text();
        if (text) message = text;
      } catch {
        /* ignore */
      }
      throw new Error(message);
    }
    const data = (await response.json()) as OrderResponse;
    if (!data || typeof data.id !== 'string') {
      throw new Error('Order service returned an unexpected response.');
    }
    return data;
  }, [buildHeaders]);

  const callIntegrationGateway = useCallback(async (orderId: string, method: PaymentMethod, amount: number): Promise<{ payment?: PaymentResponse; paymentError?: string }> => {
    if (method === 'cash') {
      return { payment: { status: 'paid' } };
    }
    try {
      const headers = buildHeaders();
      const response = await fetch(`${INTEGRATION_GATEWAY_URL}/payments`, {
        method: 'POST',
        headers,
        body: JSON.stringify({ orderId, method, amount }),
      });
      if (!response.ok) {
        let message = `Payment processing failed (${response.status})`;
        try {
          const text = await response.text();
          if (text) message = text;
        } catch {
          /* ignore */
        }
        return { paymentError: message };
      }
      const payload = (await response.json()) as PaymentResponse;
      return { payment: payload };
    } catch (err) {
      return { paymentError: parseError(err) };
    }
  }, [buildHeaders]);

  const addRecentOrder = useCallback((entry: OrderHistoryEntry) => {
    setRecentOrders(current => {
      const withoutDuplicate = current.filter(item => item.reference !== entry.reference && item.tempId !== entry.tempId);
      const next = [entry, ...withoutDuplicate];
      return next.slice(0, RECENT_ORDERS_LIMIT);
    });
  }, []);

  const updateRecentOrder = useCallback((reference: string, updater: (existing: OrderHistoryEntry) => OrderHistoryEntry): boolean => {
    let updated = false;
    setRecentOrders(current => {
      const idx = current.findIndex(item => item.tempId === reference || item.reference === reference || item.id === reference);
      if (idx === -1) return current;
      const copy = [...current];
      copy[idx] = updater(copy[idx]);
      updated = true;
      return copy;
    });
    return updated;
  }, []);

  const queueOrder = useCallback((payload: DraftOrderPayload, reason?: string) => {
    const timestamp = Date.now();
    const tempId = `offline-${timestamp}-${Math.random().toString(36).slice(2, 8)}`;
    const entry: QueuedOrder = {
      tempId,
      createdAt: timestamp,
      payload: { ...payload, offline: true },
      attempts: 0,
      lastError: reason,
    };
    setQueuedOrders(current => [...current, entry]);
    addRecentOrder({
      id: tempId,
      reference: tempId,
      status: 'Queued (offline)',
      paymentStatus: reason ? `Awaiting sync (${reason})` : 'Awaiting sync',
      paymentMethod: payload.payment_method,
      total: payload.total,
      createdAt: timestamp,
      offline: true,
      tempId,
      note: reason,
    });
    return entry;
  }, [addRecentOrder]);

  const processQueuedEntry = useCallback(async (entry: QueuedOrder): Promise<boolean> => {
    try {
      const order = await postOrder({ ...entry.payload, offline: true });
      const paymentOutcome = await callIntegrationGateway(order.id, entry.payload.payment_method, entry.payload.total);
      setQueuedOrders(current => current.filter(item => item.tempId !== entry.tempId));
      const didUpdate = updateRecentOrder(entry.tempId, existing => ({
        ...existing,
        id: order.id,
        reference: order.id,
        status: order.status ?? 'Submitted',
        paymentStatus: paymentOutcome.paymentError
          ? `Payment error: ${paymentOutcome.paymentError}`
          : paymentOutcome.payment?.status ?? (entry.payload.payment_method === 'cash' ? 'paid' : existing.paymentStatus),
        offline: false,
        tempId: undefined,
        paymentUrl: paymentOutcome.payment?.payment_url ?? paymentOutcome.payment?.paymentUrl ?? existing.paymentUrl ?? null,
        note: paymentOutcome.paymentError,
        syncedAt: Date.now(),
      }));
      if (!didUpdate) {
        addRecentOrder({
          id: order.id,
          reference: order.id,
          status: order.status ?? 'Submitted',
          paymentStatus: paymentOutcome.paymentError
            ? `Payment error: ${paymentOutcome.paymentError}`
            : paymentOutcome.payment?.status ?? (entry.payload.payment_method === 'cash' ? 'paid' : undefined),
          paymentMethod: entry.payload.payment_method,
          total: entry.payload.total,
          createdAt: entry.createdAt,
          offline: false,
          paymentUrl: paymentOutcome.payment?.payment_url ?? paymentOutcome.payment?.paymentUrl ?? null,
          note: paymentOutcome.paymentError,
          syncedAt: Date.now(),
        });
      }
      return true;
    } catch (err) {
      const message = parseError(err);
      setQueuedOrders(current => current.map(item => (item.tempId === entry.tempId ? { ...item, attempts: item.attempts + 1, lastError: message } : item)));
      const didUpdateFailure = updateRecentOrder(entry.tempId, existing => ({
        ...existing,
        note: message,
        paymentStatus: `Sync failed: ${message}`,
        status: 'Queued (offline)',
        offline: true,
      }));
      if (!didUpdateFailure) {
        addRecentOrder({
          id: entry.tempId,
          reference: entry.tempId,
          status: 'Queued (offline)',
          paymentStatus: `Sync failed: ${message}`,
          paymentMethod: entry.payload.payment_method,
          total: entry.payload.total,
          createdAt: entry.createdAt,
          offline: true,
          tempId: entry.tempId,
          note: message,
        });
      }
      return false;
    }
  }, [addRecentOrder, callIntegrationGateway, postOrder, updateRecentOrder]);

  const flushQueue = useCallback(async () => {
    if (!isBrowser) return;
    const navigatorOffline = typeof navigator !== 'undefined' && navigator.onLine === false;
    if (!isOnlineRef.current && navigatorOffline) return;
    if (queueRef.current.length === 0) return;
    if (!tenantId) return;
    if (isSyncing) return;
    setIsSyncing(true);
    try {
      for (const entry of queueRef.current) {
        const success = await processQueuedEntry(entry);
        if (!success) break;
      }
    } finally {
      setIsSyncing(false);
    }
  }, [isSyncing, processQueuedEntry, tenantId]);

  useEffect(() => {
    if (!isBrowser) return;
    if (!isOnline) {
      autoFlushRef.current = 0;
      return;
    }
    if (isSyncing) return;
    if (queueRef.current.length === 0) {
      autoFlushRef.current = 0;
      return;
    }
    const now = Date.now();
    if (now - autoFlushRef.current < AUTO_FLUSH_MIN_INTERVAL_MS) return;
    autoFlushRef.current = now;
    flushQueue().catch(err => console.warn('Unable to flush queue while online', err));
  }, [flushQueue, isOnline, isSyncing, queuedOrders.length]);

  useEffect(() => {
    if (!isBrowser) return;
    const handleOnline = () => {
      setIsOnline(true);
      flushQueue().catch(err => console.warn('Unable to flush queue', err));
    };
    const handleOffline = () => setIsOnline(false);
    window.addEventListener('online', handleOnline);
    window.addEventListener('offline', handleOffline);
    return () => {
      window.removeEventListener('online', handleOnline);
      window.removeEventListener('offline', handleOffline);
    };
  }, [flushQueue]);

  const refreshOrderStatuses = useCallback(async () => {
    if (!tenantId || !isOnline) return;
    const candidates = recentOrders.filter(shouldMonitorOrder);
    if (candidates.length === 0) return;

    const headers = buildHeaders();
    const updates = new Map<string, { status?: string; paymentStatus?: string; paymentUrl?: string | null; note?: string; syncedAt: number }>();

    const tasks = candidates.map(async order => {
      try {
        const response = await fetch(`${ORDER_SERVICE_URL}/orders/${order.reference}`, { headers });
        if (!response.ok) {
          throw new Error(`Unable to refresh order ${order.reference} (${response.status})`);
        }
        const data = (await response.json()) as OrderResponse;
        const statusUpdate = typeof data.status === 'string' ? data.status : order.status;
        const paymentUpdate = typeof data.payment_status === 'string'
          ? data.payment_status
          : typeof data.paymentStatus === 'string'
            ? data.paymentStatus
            : order.paymentStatus;
        const paymentUrlUpdate = typeof data.payment_url === 'string'
          ? data.payment_url
          : typeof data.paymentUrl === 'string'
            ? data.paymentUrl
            : order.paymentUrl ?? null;
        const noteUpdate = typeof data.note === 'string' ? data.note : order.note;
        updates.set(order.reference, {
          status: statusUpdate,
          paymentStatus: paymentUpdate,
          paymentUrl: paymentUrlUpdate,
          note: noteUpdate,
          syncedAt: Date.now(),
        });
      } catch (err) {
        console.warn('Unable to refresh order status', err);
      }
    });

    await Promise.allSettled(tasks);

    if (updates.size === 0) return;

    setRecentOrders(current =>
      current.map(entry => {
        const update = updates.get(entry.reference);
        if (!update) return entry;
        return {
          ...entry,
          status: update.status ?? entry.status,
          paymentStatus: update.paymentStatus ?? entry.paymentStatus,
          paymentUrl: update.paymentUrl ?? entry.paymentUrl ?? null,
          note: update.note ?? entry.note,
          offline: false,
          syncedAt: update.syncedAt,
        };
      }),
    );
  }, [buildHeaders, isOnline, recentOrders, tenantId]);

  useEffect(() => {
    if (!isOnline) return;
    if (!recentOrders.some(shouldMonitorOrder)) return;

    let cancelled = false;

    const run = () => {
      if (cancelled) return;
      refreshOrderStatuses().catch(err => console.warn('Unable to refresh order statuses', err));
    };

    run();
    const id = window.setInterval(run, STATUS_POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [isOnline, recentOrders, refreshOrderStatuses]);

  useEffect(() => {
    if (!isBrowser) return;
    if (!ORDER_STATUS_WS_URL) return;
    if (!isOnline || !tenantId) return;

    let didUnmount = false;

    const url = new URL(ORDER_STATUS_WS_URL);
    url.searchParams.set('tenantId', tenantId);
    if (token) url.searchParams.set('token', token);

    const socket = new WebSocket(url.toString());
    wsRef.current = socket;

    socket.onmessage = event => {
      try {
        const payload = JSON.parse(event.data);
        const orderIdCandidate: unknown = payload?.orderId ?? payload?.id ?? payload?.order_id;
        if (typeof orderIdCandidate !== 'string' || orderIdCandidate.length === 0) return;
        const statusUpdate: string | undefined = typeof payload?.status === 'string' ? payload.status : undefined;
        const paymentUpdate: string | undefined = typeof payload?.paymentStatus === 'string'
          ? payload.paymentStatus
          : typeof payload?.payment_status === 'string'
            ? payload.payment_status
            : undefined;
        const paymentUrlUpdate: string | undefined = typeof payload?.paymentUrl === 'string'
          ? payload.paymentUrl
          : typeof payload?.payment_url === 'string'
            ? payload.payment_url
            : undefined;
        const noteUpdate: string | undefined = typeof payload?.note === 'string'
          ? payload.note
          : typeof payload?.message === 'string'
            ? payload.message
            : undefined;

        const didUpdate = updateRecentOrder(orderIdCandidate, existing => ({
          ...existing,
          status: statusUpdate ?? existing.status,
          paymentStatus: paymentUpdate ?? existing.paymentStatus,
          paymentUrl: paymentUrlUpdate ?? existing.paymentUrl,
          note: noteUpdate ?? existing.note,
          offline: false,
          syncedAt: Date.now(),
        }));

        if (!didUpdate && statusUpdate) {
          console.debug('Received status for unknown order', orderIdCandidate);
        }
      } catch (err) {
        console.warn('Unable to process order status message', err);
      }
    };

    socket.onclose = () => {
      if (didUnmount) return;
      wsRef.current = null;
      if (wsTimeoutRef.current) {
        window.clearTimeout(wsTimeoutRef.current);
      }
      wsTimeoutRef.current = window.setTimeout(() => {
        if (!didUnmount) {
          setWsRetryToken(prev => prev + 1);
        }
      }, 5000);
    };

    socket.onerror = () => {
      socket.close();
    };

    return () => {
      didUnmount = true;
      if (wsTimeoutRef.current) {
        window.clearTimeout(wsTimeoutRef.current);
        wsTimeoutRef.current = null;
      }
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      } else {
        socket.close();
      }
    };
  }, [isOnline, tenantId, token, wsRetryToken, updateRecentOrder]);

  const submitOrder = useCallback(async (payload: DraftOrderPayload, options?: SubmitOptions): Promise<SubmitOrderResult> => {
    const shouldForceOffline = options?.forceOffline;
    if (!tenantId) {
      throw new Error('Tenant context is unavailable. Please log in again.');
    }
    if (payload.items.length === 0) {
      throw new Error('Cannot submit an empty order.');
    }
    const normalizedTotal = Number(payload.total);
    if (!Number.isFinite(normalizedTotal)) {
      throw new Error('Order total is invalid.');
    }

    const basePayload: DraftOrderPayload = {
      ...payload,
      total: normalizedTotal,
    };

    if (shouldForceOffline || !isOnline) {
      const queued = queueOrder(basePayload, shouldForceOffline ? 'Forced offline' : undefined);
      return {
        status: 'queued',
        tempId: queued.tempId,
        queuedCount: queuedOrders.length + 1,
      };
    }

    let order: OrderResponse;
    try {
      order = await postOrder({ ...basePayload, offline: false });
    } catch (err) {
      const message = parseError(err);
      const queued = queueOrder(basePayload, message);
      return {
        status: 'queued',
        tempId: queued.tempId,
        queuedCount: queuedOrders.length + 1,
      };
    }

    const paymentOutcome = await callIntegrationGateway(order.id, basePayload.payment_method, basePayload.total);

    addRecentOrder({
      id: order.id,
      reference: order.id,
      status: order.status ?? 'Submitted',
      paymentStatus: paymentOutcome.paymentError
        ? `Payment error: ${paymentOutcome.paymentError}`
        : paymentOutcome.payment?.status ?? (basePayload.payment_method === 'cash' ? 'paid' : undefined),
      paymentMethod: basePayload.payment_method,
      total: basePayload.total,
      createdAt: Date.now(),
      offline: false,
      paymentUrl: paymentOutcome.payment?.payment_url ?? paymentOutcome.payment?.paymentUrl ?? null,
      note: paymentOutcome.paymentError,
      syncedAt: Date.now(),
    });

    return {
      status: 'submitted',
      order,
      payment: paymentOutcome.payment,
      paymentError: paymentOutcome.paymentError,
    };
  }, [addRecentOrder, callIntegrationGateway, isOnline, postOrder, queueOrder, queuedOrders.length, tenantId]);

  const value = useMemo<OrderContextValue>(() => ({
    submitOrder,
    queuedOrders,
    recentOrders,
    isOnline,
    isSyncing,
    retryQueue: flushQueue,
    refreshOrderStatuses,
  }), [flushQueue, isOnline, isSyncing, queuedOrders, recentOrders, refreshOrderStatuses, submitOrder]);

  return <OrderContext.Provider value={value}>{children}</OrderContext.Provider>;
};

export const useOrders = (): OrderContextValue => {
  const ctx = useContext(OrderContext);
  if (!ctx) throw new Error('useOrders must be used within OrderProvider');
  return ctx;
};
