import { createHmac } from 'node:crypto';
import { execSync } from 'node:child_process';
import path from 'node:path';
import { expect, test } from '@playwright/test';

const REPO_ROOT = path.resolve(process.cwd(), '..', '..');
const ADMIN_EMAIL = 'admin@novapos.local';
const ADMIN_PASSWORD = 'admin123';
const MFA_SECRET = 'JBSWY3DPEHPK3PXP';
const CUSTOMER_SERVICE_BASE = 'http://localhost:8089';
const AUTH_SERVICE_BASE = 'http://localhost:8085';
const TENANT_ID = '00000000-0000-0000-0000-000000000001';

type CustomerRecord = {
  id: string;
  name: string;
  email: string | null;
  phone: string | null;
  created_at: string;
};

type AuditEvent = {
  timestamp: string;
  action: string;
  actor: string;
  details: string;
};

type LoginUser = {
  id: string;
  tenant_id: string;
  name?: string;
  email?: string;
  role?: string;
  roles?: string[];
  [key: string]: unknown;
};

type LoginResponse = {
  token: string;
  access_token?: string;
  user: LoginUser;
};

type ParsedCookie = {
  name: string;
  value: string;
  path?: string;
  domain?: string;
  secure?: boolean;
  httpOnly?: boolean;
  sameSite?: 'Strict' | 'Lax' | 'None';
  expires?: number;
};

type BrowserCookie = {
  name: string;
  value: string;
  path: string;
  domain?: string;
  url?: string;
  secure: boolean;
  httpOnly: boolean;
  sameSite?: 'Strict' | 'Lax' | 'None';
  expires?: number;
};

const FIXTURES = {
  customers: [
    {
      id: 'cust-1',
      name: 'Alice Smith',
      email: 'alice@example.com',
      phone: null as string | null,
      created_at: '2025-10-02T10:15:00.000Z',
    },
  ],
  auditEvents: [
    {
      timestamp: '2025-10-02T09:00:00.000Z',
      action: 'Customer Created',
      actor: 'Dana Admin',
      details: 'Initial profile created.',
    },
  ],
};

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
    case 'strict':
      return 'Strict';
    case 'lax':
      return 'Lax';
    case 'none':
      return 'None';
    default:
      return undefined;
  }
};

const parseSetCookieHeader = (raw: string): ParsedCookie | null => {
  const [nameValue, ...attributeParts] = raw.split(';');
  if (!nameValue) return null;

  const equalsIndex = nameValue.indexOf('=');
  if (equalsIndex <= 0) return null;

  const name = nameValue.slice(0, equalsIndex).trim();
  const value = nameValue.slice(equalsIndex + 1).trim();

  if (name.length === 0) return null;

  const cookie: ParsedCookie = { name, value };

  for (const attribute of attributeParts) {
    const trimmed = attribute.trim();
    if (trimmed.length === 0) continue;

    const [attributeNameRaw, ...attributeValueParts] = trimmed.split('=');
    const attributeName = attributeNameRaw.trim().toLowerCase();
    const attributeValue = attributeValueParts.join('=').trim();

    switch (attributeName) {
      case 'path':
        if (attributeValue.length > 0) {
          cookie.path = attributeValue;
        }
        break;
      case 'domain':
        if (attributeValue.length > 0) {
          cookie.domain = attributeValue;
        }
        break;
      case 'secure':
        cookie.secure = true;
        break;
      case 'httponly':
        cookie.httpOnly = true;
        break;
      case 'samesite': {
        const sameSite = toSameSite(attributeValue);
        if (sameSite) {
          cookie.sameSite = sameSite;
        }
        break;
      }
      case 'expires': {
        if (attributeValue.length > 0) {
          const parsed = Date.parse(attributeValue);
          if (!Number.isNaN(parsed)) {
            cookie.expires = Math.floor(parsed / 1000);
          }
        }
        break;
      }
      case 'max-age': {
        if (attributeValue.length > 0) {
          const parsed = Number(attributeValue);
          if (Number.isFinite(parsed)) {
            const seconds = Math.max(0, Math.floor(parsed));
            cookie.expires = Math.floor(Date.now() / 1000) + seconds;
          }
        }
        break;
      }
      default:
        break;
    }
  }

  return cookie;
};

const collectCookiesFromHeaders = (
  headers: { name: string; value: string }[],
  serviceUrl: string,
): BrowserCookie[] => {
  let fallbackDomain: string | undefined;

  try {
    const parsed = new URL(serviceUrl);
    fallbackDomain = parsed.hostname || undefined;
  } catch {
    fallbackDomain = undefined;
  }

  return headers
    .filter((header) => header.name.toLowerCase() === 'set-cookie')
    .map((header) => parseSetCookieHeader(header.value))
    .filter((cookie): cookie is ParsedCookie => cookie !== null)
    .map((cookie) => {
      const normalizedPath = (cookie.path ?? '/').trim();
      const pathValue = normalizedPath.length > 0 ? normalizedPath : '/';
      const base: BrowserCookie = {
        name: cookie.name,
        value: cookie.value,
        path: pathValue,
        secure: cookie.secure ?? false,
        httpOnly: cookie.httpOnly ?? false,
        sameSite: cookie.sameSite,
        expires: cookie.expires,
      };

      if (cookie.domain && cookie.domain.length > 0) {
        return { ...base, domain: cookie.domain };
      }

      if (fallbackDomain) {
        return { ...base, domain: fallbackDomain };
      }

      return { ...base, url: serviceUrl };
    });
};

const isLoginResponse = (value: unknown): value is LoginResponse => {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Partial<LoginResponse>;
  return (
    typeof candidate?.token === 'string' &&
    typeof candidate?.user === 'object' &&
    candidate.user !== null
  );
};

test.beforeAll(() => {
  ensureAdminMfaSeeded();
});

test.describe('Customers management', () => {
  test('supports search, audit, edit, and delete flows', async ({ page }) => {
    const totpCode = generateTotp(MFA_SECRET);
    const loginResponse = await page.request.post(`${AUTH_SERVICE_BASE}/login`, {
      headers: {
        'Content-Type': 'application/json',
        'X-Tenant-ID': TENANT_ID,
      },
      data: {
        email: ADMIN_EMAIL,
        password: ADMIN_PASSWORD,
        mfaCode: totpCode,
      },
    });

    expect(
      loginResponse.ok(),
      `Login failed with status ${loginResponse.status()}`,
    ).toBeTruthy();

    const loginJson = (await loginResponse.json()) as unknown;
    expect(isLoginResponse(loginJson), 'Unexpected login response shape').toBe(true);

    const authCookies = collectCookiesFromHeaders(
      loginResponse.headersArray(),
      AUTH_SERVICE_BASE,
    );
    expect(authCookies.length, 'Expected auth-service to return cookies').toBeGreaterThan(0);
    await page.context().addCookies(authCookies);

    await page.addInitScript(
      ({ fixtures, customerBase, authBase, tenantId, session }) => {
        const state = {
          customers: structuredClone(fixtures.customers),
          audit: structuredClone(fixtures.auditEvents),
        };

        const toJsonResponse = (payload: unknown, init?: ResponseInit) =>
          new Response(JSON.stringify(payload), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
            ...init,
          });

        const originalFetch = window.fetch.bind(window);

        window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
          const url = typeof input === 'string' ? input : input.url;
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
              typeof raw === 'string'
                ? raw
                : raw
                ? await new Response(raw).text()
                : '{}';
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
      },
      {
        fixtures: FIXTURES,
        customerBase: CUSTOMER_SERVICE_BASE,
        authBase: AUTH_SERVICE_BASE,
        tenantId: TENANT_ID,
        session: loginJson,
      },
    );

    await page.goto('/home');
    await page.waitForURL('**/home');

    await page.getByRole('button', { name: 'Go to Customers' }).click();
    await page.waitForURL('**/customers');
    await expect(page.getByRole('heading', { name: 'Customers' })).toBeVisible();

    await page.getByLabel('Search Customers').fill('Alice');
    await page.getByRole('button', { name: 'Search' }).click();
    await expect(page.getByRole('cell', { name: 'Alice Smith' })).toBeVisible();

    await page.getByRole('button', { name: 'View Activity' }).click();
    await expect(page.getByText('Customer Created')).toBeVisible();
    await page.keyboard.press('Escape');

    await page.getByRole('button', { name: 'Edit' }).click();
    await page.getByLabel('Name', { exact: true }).fill('Alice Johnson');
    await page.getByLabel('Phone').fill('555-0001');
    const saveButton = page.getByRole('button', { name: 'Save Changes' });
    await saveButton.scrollIntoViewIfNeeded();
    await saveButton.evaluate((button) => (button as HTMLButtonElement).click());
    await expect(page.getByText('Customer updated successfully.')).toBeVisible();
    await expect(page.getByRole('cell', { name: 'Alice Johnson' })).toBeVisible();

    await page.getByRole('button', { name: 'Edit' }).click();
    page.once('dialog', (dialog) => dialog.accept());
    await page.getByRole('button', { name: 'Delete Customer' }).evaluate((button) => (button as HTMLButtonElement).click());
    await expect(page.getByText('Customer deleted and anonymized.')).toBeVisible();
    await expect(page.getByText(/No customers found/i)).toBeVisible();
  });
});
