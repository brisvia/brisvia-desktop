// Configuración de los tests E2E de la interfaz de Brisvia Miner.
//
// Cómo correrlos:
//   npm run test:e2e        -> corre los tests E2E de la UI (Playwright, headless)
//   npm run test:e2e:ui     -> igual pero con el visor interactivo de Playwright
//   npm run test:rust       -> corre los tests del backend (incluye el guard del wpkh en testnet y mainnet)
//   npm run test:all        -> corre backend (Rust) + frontend (E2E)
//
// Enfoque: Playwright carga el frontend real (src/renderer) servido por un servidor estático local,
// y reemplaza el backend por un mock (tests/e2e/fixtures.js) que imita las respuestas de Tauri.
// Prueba la lógica de la UI sin necesitar bitcoind ni una build compilada.
'use strict';

const { defineConfig, devices } = require('@playwright/test');

const PORT = 4599;

module.exports = defineConfig({
  testDir: './tests/e2e',
  testMatch: '**/*.spec.js',
  // La app real es un webview basado en Chromium (WebView2), así que Chromium es representativo del frontend.
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
  // Servidor estático que sirve el frontend durante la corrida.
  webServer: {
    command: `node tests/e2e/static-server.js`,
    port: PORT,
    reuseExistingServer: !process.env.CI,
    timeout: 20000,
  },
});
