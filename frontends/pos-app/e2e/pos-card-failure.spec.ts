import { test, expect } from '@playwright/test';

const TENANT_ID = process.env.TENANT_ID || '00000000-0000-0000-0000-000000000001';

test.describe('POS card failure handling', () => {
  test('shows error, does not open window, and allows retry', async ({ page }) => {
    // Seed localStorage session
    const session = {
      token: 'dev-token',
      user: { tenant_id: TENANT_ID, roles: ['cashier'] },
      timestamp: Date.now(),
    };
    await page.addInitScript((data) => {
      try { localStorage.setItem('session', JSON.stringify(data)); } catch {}
    }, session);

    // Stub window.open and record usage
    await page.addInitScript(() => {
      (window as unknown as { __openedUrls?: string[] }).__openedUrls = [];
      const originalOpen = window.open.bind(window);
      window.open = (url: string | URL | undefined) => {
        try {
          const s = typeof url === 'string' ? url : url?.toString() ?? '';
          (window as unknown as { __openedUrls?: string[] }).__openedUrls!.push(s);
        } catch {}
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

    // Mock order submission
    await page.route('**/orders', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ id: 'ord-err', status: 'submitted' }),
      });
    });

    // First attempt: payments 400
    let attempt = 0;
    await page.route('**/payments', async (route) => {
      attempt += 1;
      if (attempt === 1) {
        await route.fulfill({ status: 400, contentType: 'text/plain', body: 'Card declined' });
      } else {
        await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ status: 'pending', paymentUrl: 'https://payments.example/checkout/ord-err' }) });
      }
    });

    await page.goto('/pos');
    await page.waitForURL('**/pos');

    // Add an item
    const addButton = page.locator('.cashier-product-card__add').first();
    await expect(addButton).toBeVisible();
    await addButton.click();

    // Select Card payment
    await page.getByLabel('Select payment method').selectOption('card');

    // Submit sale (first attempt fails)
    const submit = page.getByRole('button', { name: /Submit \/ Complete Sale/i });
    await submit.click();

    // Ensure no window.open occurred
    await page.waitForTimeout(200); // small settle time
    let openedUrls = await page.evaluate(() => (window as unknown as { __openedUrls?: string[] }).__openedUrls ?? []);
    expect(openedUrls.length).toBe(0);

    // Expect an error notice/banner in the catalog card area
    const errorNotice = page.locator('.cashier-card__notice--error');
    await expect(errorNotice).toBeVisible();

  // Retry: wait past submit throttle, then click submit again; now payments succeeds and window.open should be called
  await page.waitForTimeout(1200);
    await submit.click();
    await page.waitForFunction(() => {
      const urls = (window as unknown as { __openedUrls?: string[] }).__openedUrls ?? [];
      return urls.length > 0;
    });
    openedUrls = await page.evaluate(() => (window as unknown as { __openedUrls?: string[] }).__openedUrls ?? []);
    expect(openedUrls[0]).toContain('/checkout/ord-err');
  });
});
