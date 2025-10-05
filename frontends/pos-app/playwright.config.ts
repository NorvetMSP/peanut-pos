import { defineConfig } from '@playwright/test';

const PORT = 4174; // avoid clashing with admin-portal
const HOST = 'localhost';
const BASE_URL = `http://${HOST}:${PORT}`;

export default defineConfig({
  testDir: './e2e',
  timeout: 30_000,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL: BASE_URL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  webServer: {
    command: `npm run dev -- --host ${HOST} --port ${PORT} --strictPort`,
    url: `${BASE_URL}/pos`,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
