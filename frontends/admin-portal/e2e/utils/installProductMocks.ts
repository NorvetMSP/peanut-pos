// Product + auth fetch mocks for Playwright E2E.
// NOTE: This function runs in the browser context via page.addInitScript.

export type ProductRecord = {
  id: string;
  name: string;
  price: number;
  description: string;
  active: boolean;
  image: string;
};

export function productAuthMock(params: {
  productBase: string;
  authBase: string;
  tenantId: string;
  session: unknown;
  initialProducts?: ProductRecord[];
}): void {
  const { productBase, authBase, tenantId, session } = params;
  const state: { products: ProductRecord[] } = {
    products: (params.initialProducts ?? []).map((p) => ({ ...p })),
  };

  const json = (body: unknown, init?: ResponseInit) =>
    new Response(JSON.stringify(body), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
      ...init,
    });

  const originalFetch = window.fetch.bind(window);

  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    let url: string;
    if (typeof input === 'string') url = input;
    else if (input instanceof URL) url = input.toString();
    else if (typeof Request !== 'undefined' && input instanceof Request)
      url = input.url;
    else url = (input as { url?: string })?.url ?? '';

    const method = (init?.method ?? 'GET').toUpperCase();

    // Auth session passthrough: respond with session JSON
    if (url.startsWith(`${authBase}/session`) && method === 'GET') {
      return json(session);
    }

    // Ensure X-Tenant-ID header on auth calls
    if (url.startsWith(authBase)) {
      const requestInit: RequestInit = { ...init };
      const headers = new Headers(requestInit.headers ?? {});
      headers.set('X-Tenant-ID', tenantId);
      requestInit.headers = headers;
      return originalFetch(input, requestInit);
    }

    // Product list
    if (url.startsWith(`${productBase}/products`) && method === 'GET') {
      return json(state.products);
    }

    // Create product
    if (url === `${productBase}/products` && method === 'POST') {
      const raw = init?.body;
      const text =
        typeof raw === 'string' ? raw : raw ? await new Response(raw).text() : '{}';
      const payload = JSON.parse(text) as Partial<ProductRecord> & {
        price?: number | string;
      };
      const id = `prod-${Math.random().toString(36).slice(2, 8)}`;
      const priceNum =
        typeof payload.price === 'number'
          ? payload.price
          : Number(payload.price ?? 0);
      const created: ProductRecord = {
        id,
        name: String(payload.name ?? 'New Product'),
        price: Number.isFinite(priceNum) ? priceNum : 0,
        description: String(payload.description ?? ''),
        active: true,
        image:
          typeof payload.image === 'string' && payload.image.trim().length > 0
            ? payload.image.trim()
            : 'https://placehold.co/400x300?text=No+Image',
      };
      state.products.push(created);
      return json(created, { status: 200 });
    }

    return originalFetch(input, init);
  };
}

