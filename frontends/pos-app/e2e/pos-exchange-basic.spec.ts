import { test, expect } from '@playwright/test';

// Basic Exchange flow: mock /session, /orders/sku, and /orders/:id/exchange
// Verifies UI posts payload and renders response summary

test('POS exchange basic net-collect flow', async ({ page }) => {
  const originalOrderId = '11111111-1111-4111-8111-111111111111';
  const tenantId = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';

  // Seed localStorage session so RequireAuth passes before any page script runs
  const session = { token: 'dev-token', user: { tenant_id: tenantId, roles: ['cashier'] }, timestamp: Date.now() };
  await page.addInitScript((data: unknown) => {
    try { localStorage.setItem('session', JSON.stringify(data)); } catch {}
  }, session);

  // Seed session
  await page.route('**/session', async (route: any) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ token: 'dev', user: { tenant_id: tenantId, role: 'cashier' } }),
    });
  });

  // Minimal /login HTML to avoid navigation
  await page.route('**/login', async (route: any) => {
    await route.fulfill({ status: 200, contentType: 'text/html', body: '<html></html>' });
  });

  // Mock exchange endpoint
  await page.route('**/orders/*/exchange', async (route: any) => {
    const body = await route.request().postDataJSON();
    expect(body.return_items?.length).toBeGreaterThan(0);
    expect(body.new_items?.length).toBeGreaterThan(0);
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        original_order_id: originalOrderId,
        exchange_order_id: '22222222-2222-4222-8222-222222222222',
        refund: '33333333-3333-4333-8333-333333333333',
        refunded_cents: 1000,
        new_order_total_cents: 1500,
        net_delta_cents: 500,
        net_direction: 'collect',
      }),
    });
  });

  await page.goto('/exchange?order=' + originalOrderId);

  // Fill one return and one new item
  await page.getByRole('button', { name: '+ Add' }).first().click();
  const returnProductInput = page.locator('input[placeholder="product_id"]').first();
  await returnProductInput.fill('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaab');

  await page.getByRole('button', { name: '+ Add' }).nth(1).click();
  const skuInput = page.locator('input[placeholder="sku"]').first();
  await skuInput.fill('SKU-NEW');

  await page.getByRole('button', { name: 'Submit Exchange' }).click();

  await expect(page.getByText('Net: 500 (collect)')).toBeVisible();
});
