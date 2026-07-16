// P0 journey #11 — Change language (es/en) from Settings.
// On the real COMPILED app, verifies that choosing English/Spanish in Settings really changes the
// interface language: the chosen language button becomes active and a known UI text
// (the "Billetera"/"Wallet" tab) changes with the language. It does not assume the initial language.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Journey 11 — language', () => {
  it('switches between English and Spanish and the interface responds', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);

    const navWallet = await $('[data-testid="nav-wallet"]');
    await (await $('.nav-btn[data-view="settings"]')).click();
    const langSeg = await $('#set-language');
    await langSeg.waitForDisplayed({ timeout: 10000 });

    // Choose English -> the EN button becomes active and the tab says "Wallet".
    await (await $('#set-language .seg-btn[data-lang="en"]')).click();
    await browser.waitUntil(async () => (await navWallet.getText()).trim() === 'Wallet', {
      timeout: 8000, timeoutMsg: 'the interface did not switch to English',
    });
    expect((await $('#set-language .seg-btn[data-lang="en"]'))).toBeTruthy();
    const enActive = await (await $('#set-language .seg-btn[data-lang="en"]')).getAttribute('class');
    expect(enActive.includes('active')).toBe(true);

    // Choose Spanish -> the ES button becomes active and the tab says "Billetera".
    await (await $('#set-language .seg-btn[data-lang="es"]')).click();
    await browser.waitUntil(async () => (await navWallet.getText()).trim() === 'Billetera', {
      timeout: 8000, timeoutMsg: 'the interface did not switch back to Spanish',
    });
    const esActive = await (await $('#set-language .seg-btn[data-lang="es"]')).getAttribute('class');
    expect(esActive.includes('active')).toBe(true);
  });
});
