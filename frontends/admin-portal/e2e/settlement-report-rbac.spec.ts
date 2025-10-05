import { expect, test } from '@playwright/test';
import { createHmac } from 'node:crypto';
import { execSync } from 'node:child_process';
import path from 'node:path';

const TENANT_ID = '00000000-0000-0000-0000-000000000001';
const AUTH_SERVICE_BASE = 'http://localhost:8085';
const REPO_ROOT = path.resolve(process.cwd(), '..', '..');
const ADMIN_EMAIL = 'admin@novapos.local';
const ADMIN_PASSWORD = 'admin123';
const MFA_SECRET = 'JBSWY3DPEHPK3PXP';

// Browser-executed: seed a session via /session endpoint mock
function sessionAuthMock(params: { role: string; tenantId: string; authBase: string }) {
  const { role, tenantId, authBase } = params;
  const user = { id: 'user-1', tenant_id: tenantId, email: role + '@novapos.local', roles: [role] };
  const session = { token: 'test-token', user };
  const toJsonResponse = (payload: unknown, init?: ResponseInit) =>
    new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' }, ...init });
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    let url: string;
    if (typeof input === 'string') url = input;
    else if (input instanceof URL) url = input.toString();
    else if (typeof Request !== 'undefined' && input instanceof Request) url = input.url;
    else url = (input as { url?: unknown })?.url as string;
    const method = (init?.method ?? 'GET').toUpperCase();
    if (url.startsWith(`${authBase}/session`) && method === 'GET') {
      return toJsonResponse(session);
    }
    return originalFetch(input, init);
  };
}

// Minimal report mock for the admin happy path
function settlementReportMock(params: { orderBase: string; date: string }) {
  const { orderBase, date } = params;
  const report = { date, totals: [ { method: 'cash', count: 2, amount: '34.50' }, { method: 'card', count: 1, amount: '10.00' } ] };
  const toJsonResponse = (payload: unknown, init?: ResponseInit) =>
    new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' }, ...init });
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    let url: string;
    if (typeof input === 'string') url = input;
    else if (input instanceof URL) url = input.toString();
    else if (typeof Request !== 'undefined' && input instanceof Request) url = input.url;
    else url = (input as { url?: unknown })?.url as string;
    if (url.startsWith(`${orderBase}/reports/settlement`)) {
      (window as unknown as { __reportMockHit?: boolean; __reportMockBody?: unknown }).__reportMockHit = true;
      (window as unknown as { __reportMockHit?: boolean; __reportMockBody?: unknown }).__reportMockBody = report;
      return toJsonResponse(report);
    }
    return originalFetch(input, init);
  };
}

const ORDER_SERVICE_BASE = (process.env.VITE_ORDER_SERVICE_URL ?? 'http://localhost:8084').replace(/\/$/, '');

const today = new Date().toISOString().slice(0,10);

test.describe('Settlement Report RBAC', () => {
  test('cashier cannot access settlement report', async ({ page }) => {
    await page.addInitScript(sessionAuthMock, { role: 'cashier', tenantId: TENANT_ID, authBase: AUTH_SERVICE_BASE });
    // Start at login so AuthProvider hydrates session, LoginPage redirects to /home
    await page.goto('/login');
    await page.waitForURL('**/home', { waitUntil: 'domcontentloaded' });
    // Now attempt to access restricted page; should bounce back to /home
    await page.goto('/reports/settlement');
    await page.waitForURL('**/home', { waitUntil: 'domcontentloaded' });
  });

  test('manager can view settlement report', async ({ page }) => {
    // Perform a real login like customers.spec.ts to ensure role cookies/session are consistent
    const ensureAdminMfaSeeded = () => {
      const sql = "UPDATE users SET mfa_secret='JBSWY3DPEHPK3PXP', mfa_enrolled_at=NOW(), mfa_failed_attempts=0 WHERE email='admin@novapos.local';";
      const command = `docker compose exec -T postgres psql -U novapos -d novapos -c "${sql}"`;
      execSync(command, { cwd: REPO_ROOT, stdio: 'ignore' });
    };
    const base32Decode = (value: string): Buffer => {
      const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
      let bits = '';
      for (const char of value.replace(/=+$/u, '').toUpperCase()) {
        const idx = alphabet.indexOf(char);
        if (idx >= 0) bits += idx.toString(2).padStart(5, '0');
      }
      const bytes: number[] = [];
      for (let i = 0; i + 8 <= bits.length; i += 8) {
        bytes.push(parseInt(bits.slice(i, i + 8), 2));
      }
      return Buffer.from(bytes);
    };
    const generateTotp = (secret: string, digits = 6, stepSeconds = 30): string => {
      const key = base32Decode(secret);
      const counter = Math.floor(Date.now() / 1000 / stepSeconds);
      const buffer = Buffer.alloc(8);
      buffer.writeBigUInt64BE(BigInt(counter));
      const digest = createHmac('sha1', key).update(buffer).digest();
      const offset = digest[digest.length - 1] & 0x0f;
      const code = (digest.readUInt32BE(offset) & 0x7fffffff) % (10 ** digits);
      return code.toString().padStart(digits, '0');
    };
    const toSameSite = (value: string | undefined): 'Strict' | 'Lax' | 'None' | undefined => {
      if (!value) return undefined;
      switch (value.trim().toLowerCase()) {
        case 'strict': return 'Strict';
        case 'lax': return 'Lax';
        case 'none': return 'None';
        default: return undefined;
      }
    };
    const parseSetCookieHeader = (raw: string) => {
      const [nameValue, ...attributeParts] = raw.split(';');
      if (!nameValue) return null;
      const equalsIndex = nameValue.indexOf('=');
      if (equalsIndex <= 0) return null;
      const name = nameValue.slice(0, equalsIndex).trim();
      const value = nameValue.slice(equalsIndex + 1).trim();
      if (name.length === 0) return null;
      const cookie: any = { name, value };
      for (const attribute of attributeParts) {
        const trimmed = attribute.trim();
        if (trimmed.length === 0) continue;
        const [attributeNameRaw, ...attributeValueParts] = trimmed.split('=');
        const attributeName = attributeNameRaw.trim().toLowerCase();
        const attributeValue = attributeValueParts.join('=').trim();
        switch (attributeName) {
          case 'path': if (attributeValue.length > 0) cookie.path = attributeValue; break;
          case 'domain': if (attributeValue.length > 0) cookie.domain = attributeValue; break;
          case 'secure': cookie.secure = true; break;
          case 'httponly': cookie.httpOnly = true; break;
          case 'samesite': cookie.sameSite = toSameSite(attributeValue); break;
          case 'expires': if (attributeValue.length > 0) { const parsed = Date.parse(attributeValue); if (!Number.isNaN(parsed)) cookie.expires = Math.floor(parsed / 1000); } break;
          case 'max-age': if (attributeValue.length > 0) { const parsed = Number(attributeValue); if (Number.isFinite(parsed)) cookie.expires = Math.floor(Date.now() / 1000) + Math.max(0, Math.floor(parsed)); } break;
          default: break;
        }
      }
      return cookie;
    };
    const collectCookiesFromHeaders = (headers: { name: string; value: string }[], serviceUrl: string) => {
      let fallbackDomain: string | undefined;
      try { const parsed = new URL(serviceUrl); fallbackDomain = parsed.hostname || undefined; } catch { fallbackDomain = undefined; }
      return headers
        .filter((header) => header.name.toLowerCase() === 'set-cookie')
        .map((header) => parseSetCookieHeader(header.value))
        .filter((cookie): cookie is any => cookie !== null)
        .map((cookie) => {
          const normalizedPath = (cookie.path ?? '/').trim();
          const pathValue = normalizedPath.length > 0 ? normalizedPath : '/';
          const base: any = { name: cookie.name, value: cookie.value, path: pathValue, secure: cookie.secure ?? false, httpOnly: cookie.httpOnly ?? false, sameSite: cookie.sameSite, expires: cookie.expires };
          if (cookie.domain && cookie.domain.length > 0) return { ...base, domain: cookie.domain };
          if (fallbackDomain) return { ...base, domain: fallbackDomain };
          return { ...base, url: serviceUrl };
        });
    };
    ensureAdminMfaSeeded();
    const totpCode = generateTotp(MFA_SECRET);
    const loginResponse = await page.request.post(`${AUTH_SERVICE_BASE}/login`, {
      headers: { 'Content-Type': 'application/json', 'X-Tenant-ID': TENANT_ID },
      data: { email: ADMIN_EMAIL, password: ADMIN_PASSWORD, mfaCode: totpCode },
    });
    expect(loginResponse.ok(), `Login failed with status ${loginResponse.status()}`).toBeTruthy();
    const loginJson = await loginResponse.json();
    const authCookies = collectCookiesFromHeaders(loginResponse.headersArray(), AUTH_SERVICE_BASE);
    await page.context().addCookies(authCookies);
    // Ensure app sees a session immediately and mock the report fetch
    await page.addInitScript((params) => {
      const { authBase, tenantId, session } = params as { authBase: string; tenantId: string; session: unknown };
      const toJsonResponse = (payload: unknown, init?: ResponseInit) =>
        new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' }, ...init });
      const originalFetch = window.fetch.bind(window);
      window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
        let url: string;
        if (typeof input === 'string') url = input;
        else if (input instanceof URL) url = input.toString();
        else if (typeof Request !== 'undefined' && input instanceof Request) url = input.url;
        else url = (input as { url?: unknown })?.url as string;
        const method = (init?.method ?? 'GET').toUpperCase();
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
    }, { authBase: AUTH_SERVICE_BASE, tenantId: TENANT_ID, session: loginJson });
    await page.addInitScript(settlementReportMock, { orderBase: ORDER_SERVICE_BASE, date: today });
  // Hydrate at /home then navigate via UI card
  await page.goto('/home');
  await page.waitForURL('**/home', { waitUntil: 'domcontentloaded' });
  await page.getByRole('button', { name: 'Go to Reports' }).click();
  await page.waitForURL('**/reports/settlement', { waitUntil: 'domcontentloaded' });
    // Validate access by ensuring we remain on the route
    await expect(page).toHaveURL(/\/reports\/settlement$/);
  });
});
