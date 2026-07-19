// E2E: the mining-mode box IN the Mine tab (Priority 2.B).
//
// Shows the REAL active mode and lets the user switch SOLO <-> the official pool from Mining. 1.0.9 has SOLO and
// the official pool only (no custom pools). Changing while mining stops+restarts cleanly and CONFIRMS the new
// mode really started; it NEVER falls silently from pool to solo. Uses the test network to avoid wait mode.
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock } = require('./fixtures');

async function gotoMine(page, cfg) {
  await installMock(page, Object.assign({ network: 'brisvia-test', walletReady: true, seedOnDisk: true }, cfg));
  await page.goto('/');
  await expect(page.locator('.view[data-view="wallet"]')).toBeVisible();
  await page.locator('[data-testid="nav-mine"]').click();
  await expect(page.locator('[data-testid="view-mine"]')).toBeVisible();
}

test('mode box shows the REAL active mode (SOLO by default)', async ({ page }) => {
  await gotoMine(page, { poolEnabled: true, miningMode: 'solo' });
  await expect(page.locator('[data-testid="mine-mode-active"]')).toHaveText(/SOLO/i);
});

test('pool disabled -> pool button disabled + "coming soon"', async ({ page }) => {
  await gotoMine(page, { poolEnabled: false, miningMode: 'solo' });
  await expect(page.locator('[data-testid="mode-pool"]')).toBeDisabled();
  await expect(page.locator('#mine-mode-soon')).toBeVisible();
  await expect(page.locator('[data-testid="mine-mode-active"]')).toHaveText(/SOLO/i);
});

test('SOLO -> POOL confirms the pool is really active', async ({ page }) => {
  await gotoMine(page, { poolEnabled: true, miningMode: 'solo' });
  await page.locator('[data-testid="mode-pool"]').click();
  await expect(page.locator('[data-testid="mode-confirm"]')).toContainText(/POOL/i);
  await expect(page.locator('[data-testid="mine-mode-active"]')).toHaveText(/POOL/i);
});

test('POOL -> SOLO confirms solo is really active', async ({ page }) => {
  await gotoMine(page, { poolEnabled: true, miningMode: 'pool' });
  await expect(page.locator('[data-testid="mine-mode-active"]')).toHaveText(/POOL/i); // starts in pool
  await page.locator('[data-testid="mode-solo"]').click();
  await expect(page.locator('[data-testid="mode-confirm"]')).toContainText(/SOLO/i);
  await expect(page.locator('[data-testid="mine-mode-active"]')).toHaveText(/SOLO/i);
});

test('switch WHILE mining stops+restarts and keeps mining in the new mode', async ({ page }) => {
  await gotoMine(page, { poolEnabled: true, miningMode: 'solo' });
  await page.locator('[data-testid="mine-toggle"]').click();           // start mining (solo)
  await expect(page.locator('[data-testid="mine-toggle"]')).toHaveText(/detener|stop/i);
  await page.locator('[data-testid="mode-pool"]').click();              // switch to pool
  await expect(page.locator('[data-testid="mode-confirm"]')).toContainText(/POOL/i);
  await expect(page.locator('[data-testid="mine-mode-active"]')).toHaveText(/POOL/i);
  await expect(page.locator('[data-testid="mine-toggle"]')).toHaveText(/detener|stop/i); // still mining
});

test('restart with the previously chosen mode shows it as active', async ({ page }) => {
  // Reopening with miningMode already 'pool' (persisted) must show POOL as active, not reset to solo.
  await gotoMine(page, { poolEnabled: true, miningMode: 'pool' });
  await expect(page.locator('[data-testid="mine-mode-active"]')).toHaveText(/POOL/i);
});

test('pool configured but NOT enabled -> shows SOLO active, never a silent pool claim', async ({ page }) => {
  // miningMode 'pool' but the backend has the pool off: the honest active mode is SOLO (no silent pool claim).
  await gotoMine(page, { poolEnabled: false, miningMode: 'pool' });
  await expect(page.locator('[data-testid="mine-mode-active"]')).toHaveText(/SOLO/i);
});
