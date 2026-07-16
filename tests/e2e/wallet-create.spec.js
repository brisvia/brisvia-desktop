// E2E test: create a new wallet (the wpkh bug flow that was just fixed).
//
// Covers two things:
//  1) Happy path: from "Choose a password", with a valid password, the app ADVANCES and shows the
//     12 words, without the "wpkh(): key '...' is not valid" error.
//  2) Regression guard: if the backend were to fail again with that error, the UI shows it and does NOT
//     advance. (With a mocked backend the real key is not regenerated; the real descriptor generator is
//      validated separately by the Rust `wallet_key_tests` test. This test guards the UI contract.)
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock, captureErrors, DEMO_WORDS } = require('./fixtures');

// Advances onboarding up to the "Choose a password" screen (step 'pass') to create a wallet.
async function irAContrasena(page) {
  await page.goto('/');
  // No wallet on disk -> onboarding appears on the welcome step.
  await expect(page.locator('.step[data-step="welcome"]')).toBeVisible();
  // 3 welcome slides -> move on to "create or import".
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

  // ADVANCES: the 12-words step is shown.
  await expect(page.locator('.step[data-step="seed"]')).toBeVisible();
  await expect(page.locator('#seed-grid li')).toHaveCount(12);
  await expect(page.locator('#seed-grid li').first()).toHaveText(DEMO_WORDS[0]);

  // The wpkh error does NOT appear (nor any error message on the password screen).
  await expect(page.locator('#pass-msg')).toBeHidden();
  await expect(page.locator('body')).not.toContainText('wpkh');

  // And there were no console errors throughout the whole flow.
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

  // The UI shows the error and STAYS on the password screen (it never reaches the 12 words).
  await expect(page.locator('#pass-msg')).toBeVisible();
  await expect(page.locator('#pass-msg')).toContainText('wpkh');
  await expect(page.locator('.step[data-step="pass"]')).toBeVisible();
  await expect(page.locator('.step[data-step="seed"]')).toBeHidden();
});
