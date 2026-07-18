// P0 flow #13 — Normal re-entry (close and reopen the app).
// On the real COMPILED app: a wallet is created, the app is REOPENED (reloadSession relaunches the
// binary with the SAME data folder) and it is verified that:
//   - the app returns STRAIGHT to the wallet (onboarding does not reappear, nor does it ask to create/import again);
//   - it is the SAME wallet: revealing the phrase with the password gives the SAME 12 words.
//
// Important note (not a bug): the receive address the app shows CHANGES on each startup
// (the backend requests a fresh unused address on open: good privacy practice; the previous addresses
// still belong to the same wallet). That is why the stable check for "same wallet" is the
// 12-word PHRASE (deterministic), not the displayed address.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Flow 13 — re-entry (close and reopen)', () => {
  it('reopens the app straight to the wallet and keeps the same 12-word phrase', async () => {
    harness.fromEnv();

    const seed = await harness.onboardCreate(PASSWORD);
    expect(seed.length).toBe(12);

    // Close and reopen the app (same binary, same data folder).
    await browser.reloadSession();

    // After reopening: it must return STRAIGHT to the wallet, with onboarding not reappearing.
    const walletView = await $('[data-testid="view-wallet"]');
    const welcome = await $('[data-testid="onb-welcome"]');
    await browser.waitUntil(async () => (await walletView.isDisplayed()) || (await welcome.isDisplayed()), {
      timeout: 60000, timeoutMsg: 'the app returned neither to the wallet nor to onboarding after reopening',
    });
    expect(await welcome.isDisplayed()).toBe(false); // it remembered the existing wallet
    await walletView.waitForDisplayed({ timeout: 15000 });

    // It is the SAME wallet: revealing the phrase with the password returns the same 12 words.
    await (await $('.nav-btn[data-view="settings"]')).click();
    await (await $('#set-security')).click();
    await (await $('#modal-security')).waitForDisplayed({ timeout: 10000 });
    await (await $('#sec-reveal')).click();
    const revealModal = await $('#modal-reveal');
    await revealModal.waitForDisplayed({ timeout: 10000 });
    await (await $('#reveal-pass')).setValue(PASSWORD);
    await (await $('#reveal-go')).click();

    const seedModal = await $('#modal-seed');
    await seedModal.waitForDisplayed({ timeout: 15000 });
    const grid = await $('#seed-grid-view');
    await browser.waitUntil(async () => (await grid.$$('li')).length === 12, {
      timeout: 10000, timeoutMsg: 'the revealed phrase after reopening did not show 12 words',
    });
    const revealed = [];
    for (const li of await grid.$$('li')) revealed.push((await li.getText()).trim().replace(/^\d+[.)]?\s*/, ''));
    expect(revealed.join(' ')).toBe(seed.join(' '));
  });
});
