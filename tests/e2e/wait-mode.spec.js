// Test E2E: modo espera antes del lanzamiento (1-ago-2026 15:00 UTC).
// En una build de red real abierta ANTES del lanzamiento, la billetera funciona normal, pero el botón
// de minar queda deshabilitado y se muestra el cartel de "el minado comienza el 1 de agosto".
// Nota: el test asume que se corre antes de esa fecha (que es cuando tiene sentido este chequeo).
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock, captureErrors } = require('./fixtures');

test('modo espera: el botón de minar está deshabilitado y aparece el cartel de cuenta regresiva', async ({ page }) => {
  const errors = captureErrors(page);

  // Escenario: build de red real (mainnet) + billetera lista -> arranca en Billetera, en modo espera.
  await installMock(page, { network: 'brisvia', walletReady: true, walletOnDisk: true });

  await page.goto('/');
  await expect(page.locator('.view[data-view="wallet"]')).toBeVisible();

  // Confirmamos que la app tomó el camino de red REAL (mainnet): el panel de red muestra la etiqueta real.
  const netMainLabel = await page.evaluate(() => window.I18N.t('net_panel.net_main'));
  await expect(page.locator('#nr-network')).toHaveText(netMainLabel);

  // Vamos a la vista Minar.
  await page.click('.nav-btn[data-view="mine"]');
  await expect(page.locator('.view[data-view="mine"]')).toBeVisible();

  // El botón de minar queda DESHABILITADO (no se puede minar hasta el lanzamiento).
  await expect(page.locator('#toggle')).toBeDisabled();

  // El encabezado y la insignia muestran el estado de espera.
  const waitTitle = await page.evaluate(() => window.I18N.t('wait.title'));
  await expect(page.locator('#hero-title')).toHaveText(waitTitle);
  await expect(page.locator('#state-badge')).toHaveClass(/prep/);

  // El cartel superior muestra la cuenta regresiva al 1 de agosto.
  const waitTag = await page.evaluate(() => window.I18N.t('wait.tag'));
  await expect(page.locator('#testnet-banner')).toBeVisible();
  await expect(page.locator('#tb-tag')).toHaveText(waitTag);
  await expect(page.locator('#tb-countdown')).toBeVisible();
  await expect(page.locator('#tb-countdown')).not.toHaveText('');

  expect(errors, 'no debería haber errores de consola en modo espera:\n' + errors.join('\n')).toEqual([]);
});
