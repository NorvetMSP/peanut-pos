import { test, expect } from '@playwright/test';

const TENANT_ID = process.env.TENANT_ID || '00000000-0000-0000-0000-000000000001';

test.describe('POS card redirect', () => {
  test('submits order and opens payment URL in new tab', async ({ page }) => {
    // Seed localStorage session
    const session = {
      token: 'dev-token',
      user: { tenant_id: TENANT_ID, roles: ['cashier'] },
      timestamp: Date.now(),
    };
    await page.addInitScript((data) => {
      try { localStorage.setItem('session', JSON.stringify(data)); } catch {}
    }, session);

    // Stub window.open to capture URL
    await page.addInitScript(() => {
      (window as unknown as { __openedUrls?: string[] }).__openedUrls = [];
      const originalOpen = window.open.bind(window);
  window.open = (url: string | URL | undefined, _target?: string | undefined) => {
        try {
          const s = typeof url === 'string' ? url : url?.toString() ?? '';
          (window as unknown as { __openedUrls?: string[] }).__openedUrls!.push(s);
        } catch {}
        // do not actually open a tab during tests
        return null as unknown as Window | null;
      };
      (window as unknown as { __originalOpen?: typeof originalOpen }).__originalOpen = originalOpen;
    });

    // Mock catalog to populate grid
    await page.route('**/products', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          { id: 'p1', name: 'Gift Card', price: 25, sku: 'SKU-GC', category: 'Cards' },
        ]),
      });
    });

    // Mock order submission to include a paymentUrl
    await page.route('**/orders', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          id: 'ord-123',
          status: 'submitted',
          payment: { paymentUrl: 'https://payments.example/checkout/ord-123' },
        }),
      });
    });

    // Mock integration gateway payment creation to return a paymentUrl
    await page.route('**/payments', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ status: 'pending', paymentUrl: 'https://payments.example/checkout/ord-123' }),
      });
    });

    await page.goto('/pos');
    await page.waitForURL('**/pos');

    // Add an item
    const addButton = page.locator('.cashier-product-card__add').first();
    await expect(addButton).toBeVisible();
    await addButton.click();

    // Select Card payment
    await page.getByLabel('Select payment method').selectOption('card');

    // Submit sale
    const submit = page.getByRole('button', { name: /Submit \/ Complete Sale/i });
    await expect(submit).toBeVisible();
    await submit.click();

    // Assert window.open called with payment URL (wait for async submission)
    await page.waitForFunction(() => {
      const urls = (window as unknown as { __openedUrls?: string[] }).__openedUrls ?? [];
      return urls.length > 0;
    });
    const openedUrls = await page.evaluate(() => (window as unknown as { __openedUrls?: string[] }).__openedUrls ?? []);
    expect(openedUrls[0]).toContain('/checkout/ord-123');
  });
});
