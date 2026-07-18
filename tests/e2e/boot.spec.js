// E2E test: the app starts without errors.
// Verifies that, with an existing wallet on the real network, the app opens straight to the Wallet
// and throws no console errors or exceptions during startup and the first refresh.
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock, captureErrors } = require('./fixtures');

test('the app boots into the Wallet with no console errors', async ({ page }) => {
  const errors = captureErrors(page);

  // Scenario: real network (mainnet) + wallet already created -> starts in the Wallet view.
  await installMock(page, { network: 'brisvia', walletReady: true, walletOnDisk: true });

  await page.goto('/');

  // The Wallet view stays visible (onboarding remains hidden).
  await expect(page.locator('.view[data-view="wallet"]')).toBeVisible();
  await expect(page.locator('#setup')).toBeHidden();

  // The version chip is filled from app_version (successful startup of the bridge with the backend).
  await expect(page.locator('#ver-chip')).toHaveText('v1.0.0');

  // We give the periodic polls (node, miner, achievements) a few seconds so no late errors appear.
  await page.waitForTimeout(2500);

  expect(errors, 'there should be no console errors at startup:\n' + errors.join('\n')).toEqual([]);
});
