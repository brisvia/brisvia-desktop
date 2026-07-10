// Test E2E: crear billetera nueva (el flujo del bug del wpkh que se acaba de arreglar).
//
// Cubre dos cosas:
//  1) Camino feliz: desde "Elige una contraseña", con contraseña válida, la app AVANZA y muestra las
//     12 palabras, sin el error "wpkh(): key '...' is not valid".
//  2) Guard de regresión: si el backend volviera a fallar con ese error, la UI lo muestra y NO avanza.
//     (Con backend mockeado no se regenera la llave real; el generador real de descriptores se valida
//      aparte con el test de Rust `wallet_key_tests`. Este test cuida el contrato de la UI.)
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock, captureErrors, DEMO_WORDS } = require('./fixtures');

// Avanza el onboarding hasta la pantalla "Elige una contraseña" (paso 'pass') para crear billetera.
async function irAContrasena(page) {
  await page.goto('/');
  // Sin billetera en disco -> aparece el alta (onboarding) en el paso de bienvenida.
  await expect(page.locator('.step[data-step="welcome"]')).toBeVisible();
  // 3 diapositivas de bienvenida -> pasar a "crear o importar".
  await page.click('#onb-next');
  await page.click('#onb-next');
  await page.click('#onb-next');
  await expect(page.locator('.step[data-step="choose"]')).toBeVisible();
  // Elegir "Crear billetera" -> pantalla de contraseña.
  await page.click('#btn-create');
  await expect(page.locator('.step[data-step="pass"]')).toBeVisible();
}

test('crear billetera: contraseña válida avanza a las 12 palabras sin el error del wpkh', async ({ page }) => {
  const errors = captureErrors(page);

  // Escenario: build de red real, sin billetera -> onboarding. "Crear" devuelve 12 palabras (éxito).
  await installMock(page, {
    network: 'brisvia',
    walletReady: false,
    walletOnDisk: false,
    createWords: DEMO_WORDS,
  });

  await irAContrasena(page);

  // Contraseña válida (>= 8, coincide en ambos campos).
  await page.fill('#pass-1', 'Brisvia-Test-123');
  await page.fill('#pass-2', 'Brisvia-Test-123');
  await page.click('#pass-next');

  // AVANZA: se muestra el paso de las 12 palabras.
  await expect(page.locator('.step[data-step="seed"]')).toBeVisible();
  await expect(page.locator('#seed-grid li')).toHaveCount(12);
  await expect(page.locator('#seed-grid li').first()).toHaveText(DEMO_WORDS[0]);

  // NO aparece el error del wpkh (ni ningún mensaje de error en la pantalla de contraseña).
  await expect(page.locator('#pass-msg')).toBeHidden();
  await expect(page.locator('body')).not.toContainText('wpkh');

  // Y no hubo errores de consola en todo el flujo.
  expect(errors, 'no debería haber errores de consola al crear la billetera:\n' + errors.join('\n')).toEqual([]);
});

test('guard de regresión: si el backend devuelve el error del wpkh, la UI lo muestra y NO avanza', async ({ page }) => {
  // Escenario: "crear" FALLA con el mensaje exacto del bug histórico.
  const WPKH_ERR = "wpkh(): key 'tprv8ZgxMBicQKsPd...' is not valid";
  await installMock(page, {
    network: 'brisvia',
    walletReady: false,
    walletOnDisk: false,
    createError: WPKH_ERR,
  });

  await irAContrasena(page);

  await page.fill('#pass-1', 'Brisvia-Test-123');
  await page.fill('#pass-2', 'Brisvia-Test-123');
  await page.click('#pass-next');

  // La UI muestra el error y se QUEDA en la pantalla de contraseña (no llega a las 12 palabras).
  await expect(page.locator('#pass-msg')).toBeVisible();
  await expect(page.locator('#pass-msg')).toContainText('wpkh');
  await expect(page.locator('.step[data-step="pass"]')).toBeVisible();
  await expect(page.locator('.step[data-step="seed"]')).toBeHidden();
});
