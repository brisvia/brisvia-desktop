// E2E test: wait mode before launch (2026-08-01 15:00 UTC).
// In a real-network build opened BEFORE launch, the wallet works normally, but the mine
// button stays disabled and the notice "mining begins on August 1st" is shown.
// Note: the test assumes it runs before that date (which is when this check makes sense).
'use strict';

const { test, expect } = require('@playwright/test');
const { installMock, captureErrors } = require('./fixtures');

test('wait mode: the mine button is disabled and the countdown notice appears', async ({ page }) => {
  const errors = captureErrors(page);

  // Scenario: real-network build (mainnet) + wallet ready -> starts in the Wallet, in wait mode.
  await installMock(page, { network: 'brisvia', walletReady: true, walletOnDisk: true });

  await page.goto('/');
  await expect(page.locator('.view[data-view="wallet"]')).toBeVisible();

  // We confirm the app took the REAL-network path (mainnet): the network panel shows the real label.
  const netMainLabel = await page.evaluate(() => window.I18N.t('net_panel.net_main'));
  await expect(page.locator('#nr-network')).toHaveText(netMainLabel);

  // We go to the Mine view.
  await page.click('.nav-btn[data-view="mine"]');
  await expect(page.locator('.view[data-view="mine"]')).toBeVisible();

  // The mine button stays DISABLED (mining is not possible until launch).
  await expect(page.locator('#toggle')).toBeDisabled();

  // The heading and the badge show the wait state.
  const waitTitle = await page.evaluate(() => window.I18N.t('wait.title'));
  await expect(page.locator('#hero-title')).toHaveText(waitTitle);
  await expect(page.locator('#state-badge')).toHaveClass(/prep/);

  // The top notice shows the countdown to August 1st.
  const waitTag = await page.evaluate(() => window.I18N.t('wait.tag'));
  await expect(page.locator('#testnet-banner')).toBeVisible();
  await expect(page.locator('#tb-tag')).toHaveText(waitTag);
  await expect(page.locator('#tb-countdown')).toBeVisible();
  await expect(page.locator('#tb-countdown')).not.toHaveText('');

  expect(errors, 'there should be no console errors in wait mode:\n' + errors.join('\n')).toEqual([]);
});
