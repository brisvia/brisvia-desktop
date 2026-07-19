// E2E regressions for the pre-freeze gate (ChatGPT): A (unknown crypto), B (legacy corrupt), D (retry).
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock } = require('./fixtures');

async function gotoWallet(page, cfg) {
  await installMock(page, Object.assign({ network: 'brisvia-test', walletReady: true, seedOnDisk: true }, cfg));
  await page.goto('/');
  await expect(page.locator('.view[data-view="wallet"]')).toBeVisible();
}

// B: a CORRUPT legacy backup must go to RECOVERY (restore), never to create, and never load as a broken wallet.
test('B: corrupt legacy backup -> recovery (import), never create', async ({ page }) => {
  await installMock(page, { network: 'brisvia-test', seedOnDisk: true, walletReady: false, legacyStatus: 'legacy_corrupt' });
  await page.goto('/');
  // Onboarding shows the IMPORT/restore step, not the welcome/create step.
  await expect(page.locator('#setup')).toBeVisible();
  await expect(page.locator('#setup [data-step="import"]')).toBeVisible();
  await expect(page.locator('#setup [data-step="welcome"]')).toBeHidden();
});

// B: a VALID legacy wallet (or a modern one) does NOT get sent to recovery — normal flow.
test('B: valid/modern wallet does not trigger recovery', async ({ page }) => {
  await gotoWallet(page, { legacyStatus: 'encrypted_present' });
  await expect(page.locator('#setup')).toBeHidden();
});

// A: with the crypto state UNKNOWN (node down -> kind() fails), Send must NOT hide the password and must NOT
// allow sending; receiving/the address stay available.
test('A: unknown crypto -> Send blocked, password shown, receive still works', async ({ page }) => {
  await gotoWallet(page, { kindFails: true });
  await page.locator('[data-testid="act-send"]').click().catch(() => {});
  await page.locator('#act-send').click().catch(() => {});
  // The "cannot verify" note shows and the review button is disabled.
  await expect(page.locator('#send-crypto-unknown')).toBeVisible();
  await expect(page.locator('#send-go')).toBeDisabled();
  // The password field is NOT hidden on unknown.
  await expect(page.locator('#send-pass-field')).toBeVisible();
  // Receiving still works (address is reachable) — close send, open receive.
  await page.locator('#modal-send [data-close]').first().click().catch(() => {});
  await page.locator('#act-receive').click().catch(() => {});
  await expect(page.locator('#recv-addr')).not.toHaveText('');
});
