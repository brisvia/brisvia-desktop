// Genera capturas actualizadas del renderer 1.0.9 (mismo HTML/CSS que la app instalada) para la web,
// en ES y EN: la vista de mineria (minero en funcionamiento) y la billetera. Usa el mock de preview,
// oculta el banner de cuenta regresiva y muestra la version publica (1.0.8).
'use strict';
const { test } = require('@playwright/test');
const { installMock } = require('./fixtures');

const OUT = 'C:/Users/g43343/AppData/Local/Temp/claude/c--xampp-htdocs-crypto/1306dcf8-dc09-44e6-866c-189cf09beb65/scratchpad/shots-out';

test.use({ viewport: { width: 1024, height: 680 }, deviceScaleFactor: 2 });

test('capturas ES/EN de miner + wallet', async ({ page }) => {
  await page.addInitScript(() => {
    try { localStorage.setItem('brisvia_onboarded', '1'); localStorage.setItem('brv_lang', 'es'); } catch (e) {}
  });
  // mainnetInMs negativo = mainnet ya activa (estado real de mineria, sin modo espera ni aviso testnet).
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
