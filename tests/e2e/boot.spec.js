// Test E2E: la app arranca sin errores.
// Verifica que, con una billetera existente en la red real, la app abre directo en la Billetera
// y no tira errores de consola ni excepciones durante el arranque y el primer refresco.
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock, captureErrors } = require('./fixtures');

test('la app levanta en la Billetera sin errores de consola', async ({ page }) => {
  const errors = captureErrors(page);

  // Escenario: red real (mainnet) + billetera ya creada -> arranca en la vista Billetera.
  await installMock(page, { network: 'brisvia', walletReady: true, walletOnDisk: true });

  await page.goto('/');

  // La vista Billetera queda visible (el onboarding permanece oculto).
  await expect(page.locator('.view[data-view="wallet"]')).toBeVisible();
  await expect(page.locator('#setup')).toBeHidden();

  // El chip de versión se llena desde app_version (arranque OK del puente con el backend).
  await expect(page.locator('#ver-chip')).toHaveText('v1.0.0');

  // Damos unos segundos a los polls periódicos (nodo, minero, logros) para que no aparezcan errores tardíos.
  await page.waitForTimeout(2500);

  expect(errors, 'no debería haber errores de consola en el arranque:\n' + errors.join('\n')).toEqual([]);
});
