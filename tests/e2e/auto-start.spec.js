// E2E: the hot mainnet unlock + the voluntary auto-start-at-launch (Mining tab, real-network build).
//
// Covers the behaviours that matter most:
//   - the Mine button is blocked before the launch instant and unlocks by itself the moment it passes (no reload);
//   - the auto-start option is OFF by default and only starts mining with the user's armed consent;
//   - once mainnet is live and armed, it starts once (SOLO) — or WAITS, never falling to SOLO, when the chosen
//     POOL is unavailable;
//   - arming before launch stays "ready"; cancelling never starts.
// The launch instant comes from the backend (miner_status.mainnetStartMs), so the mock moves it to test the crossing.
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock } = require('./fixtures');

// Open the Mine tab on a REAL-network (mainnet) build, where the auto-start option is meaningful.
async function gotoMineMainnet(page, cfg) {
  await installMock(page, Object.assign({ network: 'brisvia', walletReady: true, seedOnDisk: true }, cfg));
  // The repo is English-only: run these e2e in English so the assertions read in English (no bilingual regex).
  await page.addInitScript(() => { try { localStorage.setItem('brv_lang', 'en'); } catch (e) {} });
  await page.goto('/');
  await expect(page.locator('.view[data-view="wallet"]')).toBeVisible();
  await page.locator('[data-testid="nav-mine"]').click();
  await expect(page.locator('[data-testid="view-mine"]')).toBeVisible();
}

const toggle = '[data-testid="auto-start-toggle"]';
const mineBtn = '[data-testid="mine-toggle"]';
const status = '[data-testid="auto-start-status"]';

test('auto-start is OFF by default and lives in the mining box', async ({ page }) => {
  await gotoMineMainnet(page, { mainnetInMs: 3600000 }); // 1 h ahead -> wait mode
  await expect(page.locator(toggle)).toBeVisible();
  await expect(page.locator(toggle)).not.toBeChecked();
});

test('a minute before launch the mine button is blocked', async ({ page }) => {
  await gotoMineMainnet(page, { mainnetInMs: 60000 });
  await expect(page.locator(mineBtn)).toBeDisabled();
});

test('crossing the launch instant enables the button WITHOUT a reload', async ({ page }) => {
  await page.clock.install();
  await gotoMineMainnet(page, { mainnetInMs: 5000 });
  await expect(page.locator(mineBtn)).toBeDisabled();     // before the instant
  await page.clock.fastForward(7000);                     // the clock passes the launch instant
  await expect(page.locator(mineBtn)).toBeEnabled();      // unlocked on its own, no reload
});

test('opening the app AFTER the launch instant: the button is already enabled', async ({ page }) => {
  await gotoMineMainnet(page, { mainnetInMs: -1000 });     // launch already in the past
  await expect(page.locator(mineBtn)).toBeEnabled();
});

test('WITHOUT authorisation, mining never starts on its own after launch', async ({ page }) => {
  await gotoMineMainnet(page, { mainnetInMs: -1000, autoStart: false });
  // Give the 1s loop several ticks; the button must stay on "start", never flip to mining by itself.
  await page.waitForTimeout(2500);
  await expect(page.locator(mineBtn)).not.toHaveText(/stop/i); // still on "start", not mining
});

test('WITH authorisation in SOLO, mining starts once when mainnet is live and the node is ready', async ({ page }) => {
  await gotoMineMainnet(page, { mainnetInMs: -1000, autoStart: true, miningMode: 'solo', ibd: false });
  // The armed one-shot fires: the mine button flips to "stop", meaning it started.
  await expect(page.locator(mineBtn)).toHaveText(/stop/i);
});

test('arming BEFORE launch shows "ready at launch" and does NOT start yet', async ({ page }) => {
  await gotoMineMainnet(page, { mainnetInMs: 3600000, miningMode: 'solo' });
  await page.locator(toggle).check();
  await expect(page.locator(status)).toContainText(/ready to start/i);
  await expect(page.locator(mineBtn)).toBeDisabled();       // still before launch: blocked
  await expect(page.locator(mineBtn)).not.toHaveText(/stop/i); // still on "start", not mining
});

test('SOLO with a slow node: stays armed and waits for the node, does not start', async ({ page }) => {
  await gotoMineMainnet(page, { mainnetInMs: -1000, autoStart: true, miningMode: 'solo', ibd: true });
  await expect(page.locator(status)).toContainText(/waiting for the node/i);
  await expect(page.locator(mineBtn)).not.toHaveText(/stop/i); // did not start
});

test('POOL unavailable: waits for the pool and NEVER falls to SOLO', async ({ page }) => {
  await gotoMineMainnet(page, {
    mainnetInMs: -1000, autoStart: true, miningMode: 'pool', poolEnabled: true, poolSuspended: true,
  });
  // Must show "waiting for the pool", must NOT be mining, and the active mode must remain POOL (never solo).
  await expect(page.locator(status)).toContainText(/waiting for the pool/i);
  await expect(page.locator(mineBtn)).not.toHaveText(/stop/i);
  await expect(page.locator('[data-testid="mine-mode-active"]')).toHaveText(/pool/i);
});

test('cancelling before launch: the choice is cleared and nothing starts', async ({ page }) => {
  await gotoMineMainnet(page, { mainnetInMs: 3600000, miningMode: 'solo' });
  await page.locator(toggle).check();
  await expect(page.locator(status)).toContainText(/ready to start/i);
  await page.locator('[data-testid="auto-start-cancel"]').click();
  await expect(page.locator(toggle)).not.toBeChecked();
});

test('English: the armed state reads in English when the app language is English', async ({ page }) => {
  await installMock(page, { network: 'brisvia', walletReady: true, seedOnDisk: true, mainnetInMs: 3600000, miningMode: 'solo' });
  await page.addInitScript(() => { try { localStorage.setItem('brv_lang', 'en'); } catch (e) {} });
  await page.goto('/');
  await expect(page.locator('.view[data-view="wallet"]')).toBeVisible();
  await page.locator('[data-testid="nav-mine"]').click();
  await page.locator(toggle).check();
  await expect(page.locator(status)).toContainText(/ready to start at launch/i);
});
