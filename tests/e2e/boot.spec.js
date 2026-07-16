// E2E test: the app boots with no errors.
// Verifies that, with an existing wallet on the real network, the app opens straight into the Wallet
// and throws no console errors or exceptions during boot and the first refresh.
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock, captureErrors } = require('./fixtures');

test('the app boots into the Wallet with no console errors', async ({ page }) => {
  const errors = captureErrors(page);

  // Scenario: real network (mainnet) + wallet already created -> boots into the Wallet view.
  await installMock(page, { network: 'brisvia', walletReady: true, walletOnDisk: true });

  await page.goto('/');

  // The Wallet view is visible (onboarding stays hidden).
  await expect(page.locator('.view[data-view="wallet"]')).toBeVisible();
  await expect(page.locator('#setup')).toBeHidden();

  // The version chip is filled from app_version (the bridge to the backend booted OK).
  await expect(page.locator('#ver-chip')).toHaveText('v1.0.0');

  // Give the periodic polls (node, miner, achievements) a few seconds so no late errors show up.
  await page.waitForTimeout(2500);

  expect(errors, 'there should be no console errors on boot:\n' + errors.join('\n')).toEqual([]);
});
