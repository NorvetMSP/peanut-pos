import { expect, test } from '@playwright/test';

const TENANT_ID = '00000000-0000-0000-0000-000000000001';
const AUTH_SERVICE_BASE = 'http://localhost:8085';
const ORDER_SERVICE_BASE = (process.env.VITE_ORDER_SERVICE_URL ?? 'http://localhost:8084').replace(/\/$/, '');

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

// Mock settlement report endpoint; records requested dates and returns different data per date
function settlementReportDateMock(params: { orderBase: string; today: string; other: string }) {
  const { orderBase, today, other } = params;
  const hits: string[] = [];
  const dataByDate: Record<string, any> = {
    [today]: { date: today, totals: [ { method: 'cash', count: 2, amount: '34.50' }, { method: 'card', count: 1, amount: '10.00' } ] },
    [other]: { date: other, totals: [ { method: 'cash', count: 1, amount: '5.00' }, { method: 'card', count: 3, amount: '60.00' } ] },
  };
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
      const parsed = new URL(url);
      const date = parsed.searchParams.get('date') ?? today;
      hits.push(date);
      (window as unknown as { __dateHits?: string[] }).__dateHits = hits;
      const payload = dataByDate[date] ?? dataByDate[today];
      return toJsonResponse(payload);
    }
    return originalFetch(input, init);
  };
}

test.describe('Settlement Report - date filter', () => {
  test('fetches by date and updates totals', async ({ page }) => {
    const today = new Date().toISOString().slice(0, 10);
    const other = '2025-01-01';
    await page.addInitScript(sessionAuthMock, { role: 'manager', tenantId: TENANT_ID, authBase: AUTH_SERVICE_BASE });
    await page.addInitScript(settlementReportDateMock, { orderBase: ORDER_SERVICE_BASE, today, other });

    // Hydrate at /home then navigate via UI card to ensure RequireAuth gate passes
    await page.goto('/home');
    await page.waitForURL('**/home', { waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: 'Go to Reports' }).click();
    await page.waitForURL('**/reports/settlement', { waitUntil: 'domcontentloaded' });

    // Validate initial table for today
    await expect(page.getByRole('heading', { name: 'Settlement Report' })).toBeVisible();
  const cashRow = page.locator('tbody tr').filter({ has: page.getByRole('cell', { name: 'cash', exact: true }) });
  await expect(cashRow).toBeVisible();
  await expect(cashRow.getByRole('cell').nth(0)).toHaveText('cash');
  await expect(cashRow.getByRole('cell').nth(1)).toHaveText('2');
  await expect(cashRow.getByRole('cell').nth(2)).toHaveText(/\$?34\.50/);
  const cardRow = page.locator('tbody tr').filter({ has: page.getByRole('cell', { name: 'card', exact: true }) });
  await expect(cardRow).toBeVisible();
  await expect(cardRow.getByRole('cell').nth(0)).toHaveText('card');
  await expect(cardRow.getByRole('cell').nth(1)).toHaveText('1');
  await expect(cardRow.getByRole('cell').nth(2)).toHaveText(/\$?10\.00/);

    // Change date to other and assert table updates
    const dateInput = page.locator('#report-date');
    await dateInput.fill(other);
    // Wait for the UI to update with the mocked values
  const cashRow2 = page.locator('tbody tr').filter({ has: page.getByRole('cell', { name: 'cash', exact: true }) });
  await expect(cashRow2.getByRole('cell').nth(0)).toHaveText('cash');
  await expect(cashRow2.getByRole('cell').nth(1)).toHaveText('1');
  await expect(cashRow2.getByRole('cell').nth(2)).toHaveText(/\$?5\.00/);
  const cardRow2 = page.locator('tbody tr').filter({ has: page.getByRole('cell', { name: 'card', exact: true }) });
  await expect(cardRow2.getByRole('cell').nth(0)).toHaveText('card');
  await expect(cardRow2.getByRole('cell').nth(1)).toHaveText('3');
  await expect(cardRow2.getByRole('cell').nth(2)).toHaveText(/\$?60\.00/);

    // Confirm our mock saw both dates
    const datesHit = await page.evaluate(() => (window as unknown as { __dateHits?: string[] }).__dateHits ?? []);
    expect(datesHit).toContain(today);
    expect(datesHit).toContain(other);
  });
});
