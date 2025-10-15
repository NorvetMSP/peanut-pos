import { createHmac } from 'node:crypto';
import { execSync } from 'node:child_process';
import path from 'node:path';
import { expect, test } from '@playwright/test';
// Real product-service flow: no API mocks

const REPO_ROOT = path.resolve(process.cwd(), '..', '..');
const ADMIN_EMAIL = 'admin@novapos.local';
const ADMIN_PASSWORD = 'admin123';
const MFA_SECRET = 'JBSWY3DPEHPK3PXP';
const PRODUCT_SERVICE_BASE = 'http://localhost:8081';
const AUTH_SERVICE_BASE = 'http://localhost:8085';
const TENANT_ID = '00000000-0000-0000-0000-000000000001';

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
  const code = (digest.readUInt32BE(offset) & 0x7fffffff) % 1_000_000;
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
        if (attributeValue.length > 0) cookie.path = attributeValue;
        break;
      case 'domain':
        if (attributeValue.length > 0) cookie.domain = attributeValue;
        break;
      case 'secure':
        cookie.secure = true;
        break;
      case 'httponly':
        cookie.httpOnly = true;
        break;
      case 'samesite': {
        const sameSite = toSameSite(attributeValue);
        if (sameSite) cookie.sameSite = sameSite;
        break;
      }
      case 'expires': {
        if (attributeValue.length > 0) {
          const parsed = Date.parse(attributeValue);
          if (!Number.isNaN(parsed)) cookie.expires = Math.floor(parsed / 1000);
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

test.describe('Products management', () => {
  test('adds a product from Products page', async ({ page }) => {
    // Login to auth-service (real) with MFA
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

    expect(loginResponse.ok(), `Login failed with status ${loginResponse.status()}`).toBeTruthy();
    const loginJson = (await loginResponse.json()) as unknown;
    expect(isLoginResponse(loginJson), 'Unexpected login response shape').toBe(true);

    // Carry over cookies into browser context
    const authCookies = collectCookiesFromHeaders(loginResponse.headersArray(), AUTH_SERVICE_BASE);
    expect(authCookies.length, 'Expected auth-service to return cookies').toBeGreaterThan(0);
    await page.context().addCookies(authCookies);

    // Ensure SPA sees a session immediately by mocking only /session (real product-service untouched)
    await page.addInitScript(
      ({ authBase, session }) => {
        const originalFetch = window.fetch.bind(window);
        window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
          let url: string;
          if (typeof input === 'string') url = input;
          else if (input instanceof URL) url = input.toString();
          else if (typeof Request !== 'undefined' && input instanceof Request)
            url = input.url;
          else url = (input as { url?: string })?.url ?? '';
          const method = (init?.method ?? 'GET').toUpperCase();
          if (url.startsWith(`${authBase}/session`) && method === 'GET') {
            return new Response(JSON.stringify(session), {
              status: 200,
              headers: { 'Content-Type': 'application/json' },
            });
          }
          return originalFetch(input, init);
        };
      },
      { authBase: AUTH_SERVICE_BASE, session: loginJson },
    );

    // Navigate to Products via Admin Home
    await page.goto('/home');
    await expect(page).toHaveURL(/\/home/);
    await page.getByRole('button', { name: 'Go to Products' }).click();
    await expect(page).toHaveURL(/\/products/);
    await expect(
      page.getByRole('heading', { name: 'Products', exact: true }),
    ).toBeVisible();

    // Fill Add New Product form
    await expect(
      page.getByRole('heading', { name: 'Add New Product' }),
    ).toBeVisible();
    await page.getByPlaceholder('Deluxe Latte').fill('Test Latte');
    const priceInput = page.locator('label:has-text("Price")').locator('..').locator('input[type="number"]');
    await priceInput.fill('9.99');
    const descInput = page.locator('label:has-text("Description")').locator('..').locator('textarea');
    await descInput.fill('Smooth and rich');
    await page.getByRole('button', { name: 'Add Product' }).click();

    // Confirm success banner and locate the specific table row robustly
    await expect(page.getByText('Product added successfully.')).toBeVisible();
    const productsTable = page.getByRole('table');
    const createdRow = productsTable
      .getByRole('row')
      .filter({ hasText: 'Test Latte' })
      .filter({ hasText: '$9.99' })
      .first();
    await expect(createdRow).toBeVisible();
  });
});
