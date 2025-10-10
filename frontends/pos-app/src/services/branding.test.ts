import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { resolveBranding, parseBrandingFromEnv } from './branding';

const originalEnv = (import.meta as any).env;
const originalFetch = globalThis.fetch;

function setEnv(partial: Record<string, unknown>) {
  (import.meta as any).env = { ...(originalEnv ?? {}), ...partial };
}

describe('branding resolver', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  afterEach(() => {
    // restore env and fetch for isolation
    (import.meta as any).env = originalEnv;
    globalThis.fetch = originalFetch as any;
  });

  it('parses env variables into branding config (trim + split)', () => {
    const branding = parseBrandingFromEnv({
      VITE_BRAND_NAME: '  Acme Retail  ',
      VITE_BRAND_HEADER_LINES: ' Line 1 | Line 2 |  ',
    });
    expect(branding.brandName).toBe('Acme Retail');
    expect(branding.brandHeaderLines).toEqual(['Line 1', 'Line 2']);
  });

  it('uses tenant branding API when available and merges with env fallback', async () => {
    setEnv({
      VITE_AUTH_SERVICE_URL: 'http://auth.local',
      VITE_BRAND_NAME: 'Fallback Brand',
      VITE_BRAND_HEADER_LINES: 'FromEnv',
    });

    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes('/branding')) {
        // Ensure headers are passed (auth + tenant)
        const headers = (init?.headers ?? {}) as Record<string, string>;
        expect(headers['Authorization']).toBe('Bearer tok');
        expect(headers['X-Tenant-ID']).toBe('t1');
        return new Response(
          JSON.stringify({ brand_name: 'API Brand', header_lines: ['API L1', 'API L2'] }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        );
      }
      return new Response('not found', { status: 404 });
    }) as unknown as typeof fetch;

    globalThis.fetch = fetchMock;

    const result = await resolveBranding('t1', 'tok');
    expect(result.brandName).toBe('API Brand');
    expect(result.brandHeaderLines).toEqual(['API L1', 'API L2']);
  });
});
