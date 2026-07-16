// P0 journey #6 — Close and recovery.
// On the real COMPILED app: a wallet is created, an OPERATION is opened (the send modal with data
// half filled in) and the app is CLOSED/REOPENED at that moment (reloadSession relaunches the binary
// with the same data folder). Verifies that:
//   - reopening does not corrupt the wallet: revealing the phrase with the password gives the SAME 12 words;
//   - the app reopens clean and straight into the wallet (onboarding does not reappear, nor the half modal).
// The absence of orphan processes after closing is validated by the runner (teardown -> countProcs, see run.js).
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Journey 6 — close and recovery', () => {
  it('closes during an operation and reopens without corrupting the wallet', async () => {
    harness.fromEnv();

    const seed = await harness.onboardCreate(PASSWORD);

    // 1) Open a half-done operation: the send modal with partial data, without confirming.
    await (await $('.nav-btn[data-view="wallet"]')).click();
    await (await $('#act-send')).click();
    const sendModal = await $('#modal-send');
    await sendModal.waitForDisplayed({ timeout: 10000 });
    await (await $('#send-addr')).setValue('brv1qexampleexampleexampleexample00');
    await (await $('#send-amount')).setValue('5');

    // 2) Close and reopen the app IN THE MIDDLE of the operation.
    await browser.reloadSession();

    // 3) Reopens clean and straight into the wallet (onboarding does not reappear, no half modal remains).
    const walletView = await $('[data-testid="view-wallet"]');
    const welcome = await $('[data-testid="onb-welcome"]');
    await browser.waitUntil(async () => (await walletView.isDisplayed()) || (await welcome.isDisplayed()), {
      timeout: 60000, timeoutMsg: 'the app did not come back after closing during the operation',
    });
    expect(await welcome.isDisplayed()).toBe(false);
    await walletView.waitForDisplayed({ timeout: 15000 });
    expect(await (await $('#modal-send')).isDisplayed()).toBe(false); // the half operation did not stay open

    // 4) The wallet was NOT corrupted: revealing the phrase gives the same 12 words.
    await (await $('.nav-btn[data-view="settings"]')).click();
    await (await $('#set-security')).click();
    await (await $('#modal-security')).waitForDisplayed({ timeout: 10000 });
    await (await $('#sec-reveal')).click();
    await (await $('#modal-reveal')).waitForDisplayed({ timeout: 10000 });
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
