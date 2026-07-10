// Test E2E: crear billetera nueva (el flujo del bug del wpkh que se acaba de arreglar).
//
// Cubre dos cosas:
//  1) Happy path: from "Choose a password", with a valid password, the app ADVANCES and shows the
//     12 palabras, sin el error "wpkh(): key '...' is not valid".
//  2) Regression guard: if the backend fails again with that error, the UI shows it and does NOT advance.
//     (Con backend mockeado no se regenera la llave real; el generador real de descriptores se valida
//      aparte con el test de Rust `wallet_key_tests`. Este test cuida el contrato de la UI.)
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock, captureErrors, DEMO_WORDS } = require('./fixtures');

// Advances onboarding to the "Choose a password" screen (step 'pass') to create a wallet.
async function irAContrasena(page) {
  await page.goto('/');
  // Sin billetera en disco -> aparece el alta (onboarding) en el paso de bienvenida.
  await expect(page.locator('.step[data-step="welcome"]')).toBeVisible();
  // 3 diapositivas de bienvenida -> pasar a "crear o importar".
  await page.click('#onb-next');
  await page.click('#onb-next');
  await page.click('#onb-next');
  await expect(page.locator('.step[data-step="choose"]')).toBeVisible();
  // Choose "Create wallet" -> password screen.
  await page.click('#btn-create');
  await expect(page.locator('.step[data-step="pass"]')).toBeVisible();
}

test('create wallet: a valid password advances to the 12 words without the wpkh error', async ({ page }) => {
  const errors = captureErrors(page);

  // Scenario: real-network build, no wallet -> onboarding. "Create" returns 12 words (success).
  await installMock(page, {
    network: 'brisvia',
    walletReady: false,
    walletOnDisk: false,
    createWords: DEMO_WORDS,
  });

  await irAContrasena(page);

  // Valid password (>= 8, matches in both fields).
  await page.fill('#pass-1', 'Brisvia-Test-123');
  await page.fill('#pass-2', 'Brisvia-Test-123');
  await page.click('#pass-next');

  // AVANZA: se muestra el paso de las 12 palabras.
  await expect(page.locator('.step[data-step="seed"]')).toBeVisible();
  await expect(page.locator('#seed-grid li')).toHaveCount(12);
  await expect(page.locator('#seed-grid li').first()).toHaveText(DEMO_WORDS[0]);

  // The wpkh error does NOT appear (nor any error message on the password screen).
  await expect(page.locator('#pass-msg')).toBeHidden();
  await expect(page.locator('body')).not.toContainText('wpkh');

  // Y no hubo errores de consola en todo el flujo.
  expect(errors, 'there should be no console errors when creating the wallet:\n' + errors.join('\n')).toEqual([]);
});

test('regression guard: if the backend returns the wpkh error, the UI shows it and does NOT advance', async ({ page }) => {
  // Scenario: "create" FAILS with the exact message of the historical bug.
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

  // The UI shows the error and STAYS on the password screen (does not reach the 12 words).
  await expect(page.locator('#pass-msg')).toBeVisible();
  await expect(page.locator('#pass-msg')).toContainText('wpkh');
  await expect(page.locator('.step[data-step="pass"]')).toBeVisible();
  await expect(page.locator('.step[data-step="seed"]')).toBeHidden();
});
