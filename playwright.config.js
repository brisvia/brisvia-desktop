// Configuration for Brisvia Miner's UI E2E tests.
//
// How to run them:
//   npm run test:e2e        -> runs the UI E2E tests (Playwright, headless)
//   npm run test:e2e:ui     -> same but with Playwright's interactive viewer
//   npm run test:rust       -> runs the backend tests (includes the wpkh guard on testnet and mainnet)
//   npm run test:all        -> runs backend (Rust) + frontend (E2E)
//
// Approach: Playwright loads the real frontend (src/renderer) served by a local static server,
// and replaces the backend with a mock (tests/e2e/fixtures.js) that mimics Tauri's responses.
// It tests the UI logic without needing bitcoind or a compiled build.
'use strict';

const { defineConfig, devices } = require('@playwright/test');

const PORT = 4599;

module.exports = defineConfig({
  testDir: './tests/e2e',
  testMatch: '**/*.spec.js',
  // The real app is a Chromium-based webview (WebView2), so Chromium is representative of the frontend.
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: [['list']],
  timeout: 30000,
  expect: { timeout: 8000 },
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    headless: true,
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
  ],
  // Static server that serves the frontend during the run.
  webServer: {
    command: `node tests/e2e/static-server.js`,
    port: PORT,
    reuseExistingServer: !process.env.CI,
    timeout: 20000,
  },
});
