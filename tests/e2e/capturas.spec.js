// Generates website captures of the renderer (the same HTML/CSS the installed app runs) in ES and EN:
// the mining view with the miner running, and the wallet. Uses the preview mock and hides the countdown
// banner so the shots do not go stale the day the countdown ends.
'use strict';
const path = require('path');
const { test } = require('@playwright/test');
const { installMock } = require('./fixtures');

// Writes inside the repo by default so this runs on any machine. Point SHOTS_OUT somewhere else to
// override. It used to be an absolute path from one developer's temp directory, which meant the file
// published a local username and worked nowhere but that laptop.
const OUT = process.env.SHOTS_OUT || path.join(__dirname, '..', '..', 'shots-out');

test.use({ viewport: { width: 1024, height: 680 }, deviceScaleFactor: 2 });

test('captures ES/EN of miner + wallet', async ({ page }) => {
  await page.addInitScript(() => {
    try { localStorage.setItem('brisvia_onboarded', '1'); localStorage.setItem('brv_lang', 'es'); } catch (e) {}
  });
  // negative mainnetInMs = mainnet already live (real mining state, no wait mode, no testnet notice).
  await installMock(page, { network: 'brisvia', walletReady: true, seedOnDisk: true, mainnetInMs: -1000, poolEnabled: true });
  await page.goto('/');
  await page.locator('.view[data-view="wallet"]').waitFor({ timeout: 15000 });
  await page.addStyleTag({ content: '#testnet-banner{display:none!important}' });

  async function snap(name) {
    await page.evaluate(() => { var v = document.getElementById('ver-chip'); if (v) v.textContent = 'v1.0.8'; });
    await page.mouse.move(5, 5); // move the cursor away so no hover tooltip appears over the controls
    await page.waitForTimeout(300);
    await page.screenshot({ path: OUT + '/' + name });
  }

  await page.waitForTimeout(1200);
  await snap('wallet-es.png');

  await page.locator('[data-testid="nav-mine"]').click();
  await page.locator('[data-testid="view-mine"]').waitFor();
  await page.waitForTimeout(600);
  await page.locator('[data-testid="mine-toggle"]').click().catch(() => {});
  await page.waitForTimeout(2200);
  await snap('miner-es.png');

  await page.evaluate(() => { if (window.I18N) window.I18N.setLang('en'); try { localStorage.setItem('brv_lang', 'en'); } catch (e) {} });
  await page.waitForTimeout(900);
  await snap('miner-en.png');

  await page.locator('.nav-btn[data-view="wallet"]').click();
  await page.waitForTimeout(900);
  await snap('wallet-en.png');
});
