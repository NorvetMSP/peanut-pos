import { test, expect } from '@playwright/test';

const TENANT_ID = process.env.TENANT_ID || '00000000-0000-0000-0000-000000000001';

test('POS exchange: select items from original order and submit', async ({ page }) => {
  const orderId = '11111111-1111-4111-8111-111111111111';

  // Seed local session
  const session = { token: 'dev-token', user: { tenant_id: TENANT_ID, roles: ['cashier'] }, timestamp: Date.now() };
  await page.addInitScript((data) => {
    try { localStorage.setItem('session', JSON.stringify(data)); } catch {}
  }, session);

  // Mock GET original order to return items
  await page.route(`**/orders/${orderId}`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        id: orderId,
        items: [
          { product_id: 'prod-A', name: 'Alpha', quantity: 2, returned_quantity: 1 }, // max 1 left
          { product_id: 'prod-B', name: 'Beta', quantity: 1, returned_quantity: 0 },  // max 1
        ],
      }),
    });
  });

  // Capture POST exchange
  let postedBody: any = null;
  await page.route(`**/orders/${orderId}/exchange`, async (route) => {
    postedBody = await route.request().postDataJSON();
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        original_order_id: orderId,
        exchange_order_id: '22222222-2222-4222-8222-222222222222',
        refunded_cents: 100,
        new_order_total_cents: 0,
        net_delta_cents: -100,
        net_direction: 'refund',
      }),
    });
  });

  await page.goto(`/exchange?order=${orderId}`);

  // Wait for order items UI
  await expect(page.getByText('Return from original order')).toBeVisible();

  // Set selected qty for first item
  const qtyInput = page.getByLabel('Return qty prod-A');
  await expect(qtyInput).toBeVisible();
  await qtyInput.fill('1');

  // Add selected to return list
  const addSelected = page.getByRole('button', { name: 'Add selected' });
  await addSelected.click();

  // Verify it appears in Return Items list
  await expect(page.locator('input[placeholder="product_id"]').first()).toHaveValue('prod-A');

  // Submit
  await page.getByRole('button', { name: 'Submit Exchange' }).click();

  // Assert payload shape and UI summary
  expect(Array.isArray(postedBody?.return_items)).toBeTruthy();
  const firstReturn = postedBody.return_items.find((r: any) => r.product_id === 'prod-A');
  expect(firstReturn?.qty).toBe(1);
  await expect(page.getByText('Net: -100 (refund)')).toBeVisible();
});
