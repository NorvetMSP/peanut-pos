import { test, expect } from '@playwright/test';

// Minimal POS smoke: seed session, mock network, add item, submit cash sale.

const TENANT_ID = process.env.TENANT_ID || '00000000-0000-0000-0000-000000000001';

test.describe('POS cashier smoke', () => {
  test('add item and submit sale (cash)', async ({ page }) => {
    // Seed localStorage session before any page script runs
    const session = {
      token: 'dev-token',
      user: { tenant_id: TENANT_ID, roles: ['cashier'] },
      timestamp: Date.now(),
    };
    await page.addInitScript((data) => {
      try {
        localStorage.setItem('session', JSON.stringify(data));
      } catch { /* ignore */ }
    }, session);

    // Mock product catalog and order submit
    await page.route('**/products', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          { id: 'p1', name: 'Soda Can', price: 1.99, sku: 'SKU-SODA', category: 'Drinks' },
          { id: 'p2', name: 'Bottle Water', price: 1.49, sku: 'SKU-WATER', category: 'Drinks' },
        ]),
      });
    });
    await page.route('**/orders', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ id: '11111111-1111-1111-1111-111111111111', status: 'COMPLETED' }),
      });
    });

    await page.goto('/pos');
    await page.waitForURL('**/pos');

    // Add first visible item
    const addButtons = page.locator('.cashier-product-card__add');
    await expect(addButtons.first()).toBeVisible();
    await addButtons.first().click();

  // Submit sale
  const submit = page.getByRole('button', { name: /Submit \/ Complete Sale/i });
    await expect(submit).toBeVisible();
    await submit.click();

    // UI remains responsive
    await expect(addButtons.first()).toBeVisible();
  });
});
