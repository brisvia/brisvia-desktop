// E2E: the "Share Brisvia" button on the Mining tab. Only networks with a native share URL are offered
// (X, Telegram, WhatsApp, Facebook) plus "copy link". Each opens a UNIVERSAL share URL that carries the
// brisvia.com link; no balance/address/private data is ever included, and nothing is posted automatically.
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock } = require('./fixtures');

async function gotoMine(page) {
  await installMock(page, { network: 'brisvia', walletReady: true, seedOnDisk: true, mainnetInMs: -1000, poolEnabled: true });
  await page.goto('/');
  await expect(page.locator('.view[data-view="wallet"]')).toBeVisible();
  await page.locator('[data-testid="nav-mine"]').click();
  await expect(page.locator('[data-testid="view-mine"]')).toBeVisible();
}

test('share: the button opens the modal with the four native-share networks + copy link', async ({ page }) => {
  await gotoMine(page);
  await page.locator('[data-testid="share-open"]').click();
  await expect(page.locator('[data-testid="modal-share"]')).toBeVisible();
  for (const net of ['x', 'telegram', 'whatsapp', 'facebook']) {
    await expect(page.locator(`[data-testid="share-${net}"]`)).toBeVisible();
  }
  await expect(page.locator('[data-testid="share-copy"]')).toBeVisible();
});

test('share: each network opens a universal share URL that carries the brisvia.com link', async ({ page }) => {
  await gotoMine(page);
  // Intercept openUrl so the browser is not actually launched; record what each button would open.
  await page.evaluate(() => { window.__opened = []; window.brisvia.openUrl = (u) => { window.__opened.push(u); }; });
  await page.locator('[data-testid="share-open"]').click();
  for (const net of ['x', 'telegram', 'whatsapp', 'facebook']) {
    await page.locator(`[data-testid="share-${net}"]`).click();
  }
  const opened = await page.evaluate(() => window.__opened);
  expect(opened.length).toBe(4);
  expect(opened.some((u) => u.includes('twitter.com/intent/tweet'))).toBeTruthy();
  expect(opened.some((u) => u.includes('t.me/share/url'))).toBeTruthy();
  expect(opened.some((u) => u.includes('wa.me/?text='))).toBeTruthy();
  expect(opened.some((u) => u.includes('facebook.com/sharer'))).toBeTruthy();
  // Every share must carry the brisvia.com link and NEVER any wallet/private data.
  expect(opened.every((u) => u.includes('brisvia.com'))).toBeTruthy();
  expect(opened.every((u) => !/bc1|brva1|addr|balance|seed/i.test(u))).toBeTruthy();
});
