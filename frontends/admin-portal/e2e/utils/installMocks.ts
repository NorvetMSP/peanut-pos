// Utility to install customer + auth fetch mocks inside browser context.
// NOTE: This function executes in the browser (page) context when passed to page.addInitScript.
import type { CustomerRecord, AuditEvent } from './types';

type Fixtures = { customers: CustomerRecord[]; auditEvents: AuditEvent[] };

// Exported function MUST be serializable; avoid external closures.
// It receives a single parameter with runtime values provided by Playwright.
// Keep logic synchronous except for fetch override internals.
export function customerAuthMock(params: {
  fixtures: Fixtures;
  customerBase: string;
  authBase: string;
  tenantId: string;
  session: unknown;
}): void {
  const { fixtures, customerBase, authBase, tenantId, session } = params;
  const state = {
    customers: structuredClone(fixtures.customers),
    audit: structuredClone(fixtures.auditEvents),
  };
  // Expose internal state for ordering assertions in tests.
  (window as unknown as { __mockState?: typeof state }).__mockState = state;

  const toJsonResponse = (payload: unknown, init?: ResponseInit) =>
    new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
      ...init,
    });

  const originalFetch = window.fetch.bind(window);

  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    let url: string;
    if (typeof input === 'string') {
      url = input;
    } else if (input instanceof URL) {
      url = input.toString();
    } else if (typeof Request !== 'undefined' && input instanceof Request) {
      url = input.url;
    } else {
      const possible = (input as { url?: unknown })?.url;
      url = typeof possible === 'string' ? possible : '';
    }
    const method = (init?.method ?? 'GET').toUpperCase();

    if (url.startsWith(`${customerBase}/customers/cust-1/audit`) && method === 'GET') {
      return toJsonResponse(state.audit);
    }

    if (url.startsWith(`${customerBase}/customers/cust-1/gdpr/delete`) && method === 'POST') {
      state.customers = [];
      state.audit.unshift({
        timestamp: new Date('2025-10-02T13:00:00.000Z').toISOString(),
        action: 'Customer Deleted',
        actor: 'Dana Admin',
        details: 'GDPR delete issued from UI test.',
      });
      return toJsonResponse({ status: 'deleted' });
    }

    if (url.startsWith(`${customerBase}/customers/cust-1`) && method === 'PUT') {
      const raw = init?.body;
      const text =
        typeof raw === 'string' ? raw : raw ? await new Response(raw).text() : '{}';
      // Lightweight parse + merge (type validation can be added if desired)
      const payload = JSON.parse(text) as Partial<CustomerRecord>;
      state.customers[0] = {
        ...state.customers[0],
        ...(payload.name !== undefined ? { name: payload.name } : {}),
        ...(payload.email !== undefined ? { email: payload.email } : {}),
        ...(payload.phone !== undefined ? { phone: payload.phone } : {}),
      };
      state.audit.unshift({
        timestamp: new Date('2025-10-02T12:00:00.000Z').toISOString(),
        action: 'Profile Updated',
        actor: 'Dana Admin',
        details: 'Profile edited via UI test.',
      });
      return toJsonResponse(state.customers[0]);
    }

    if (url.startsWith(`${customerBase}/customers`) && method === 'GET') {
      return toJsonResponse(state.customers);
    }

    if (url.startsWith(`${authBase}/session`) && method === 'GET') {
      return toJsonResponse(session);
    }

    if (url.startsWith(authBase)) {
      const requestInit: RequestInit = { ...init };
      const headers = new Headers(requestInit.headers ?? {});
      headers.set('X-Tenant-ID', tenantId);
      requestInit.headers = headers;
      return originalFetch(input, requestInit);
    }

    return originalFetch(input, init);
  };
}
