// E2E regression: wallet existence is independent of the node.
//
// Audit priority: the existence of the wallet MUST be resolved locally (seedOnDisk reads wallet_seed.enc)
// BEFORE consulting the node. A slow / down / syncing / offline node must NEVER open the create/restore
// onboarding on top of an existing wallet — no RPC timeout or error can show the first-run flow. And the
// opposite must still hold: with no wallet on disk, onboarding shows regardless of the node.
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock, captureErrors } = require('./fixtures');

test('existing wallet + node DOWN never shows onboarding', async ({ page }) => {
  const errors = captureErrors(page);
  // Wallet exists on disk, but the node is down (connected:false) and never loads it (walletReady:false).
  await installMock(page, { network: 'brisvia', seedOnDisk: true, walletReady: false, connected: false });
  await page.goto('/');
  // Onboarding must stay hidden: the local seedOnDisk check already proved the wallet exists.
  await page.waitForTimeout(2000);
  await expect(page.locator('#setup')).toBeHidden();
  expect(errors, 'no console errors while waiting on a down node:\n' + errors.join('\n')).toEqual([]);
});

test('existing wallet + node SLOW (not ready yet) never shows onboarding', async ({ page }) => {
  const errors = captureErrors(page);
  // Node connected but still loading the wallet (walletReady:false) — e.g. a slow start after an update.
  await installMock(page, { network: 'brisvia', seedOnDisk: true, walletReady: false, connected: true });
  await page.goto('/');
  await page.waitForTimeout(2000);
  await expect(page.locator('#setup')).toBeHidden();
  expect(errors, 'no console errors while the node loads the wallet:\n' + errors.join('\n')).toEqual([]);
});

test('NO wallet on disk shows onboarding regardless of node state', async ({ page }) => {
  // The only thing that opens onboarding is the LOCAL absence of the seed, never the node.
  await installMock(page, { network: 'brisvia', seedOnDisk: false, walletReady: false, connected: false });
  await page.goto('/');
  await expect(page.locator('#setup')).toBeVisible();
});
