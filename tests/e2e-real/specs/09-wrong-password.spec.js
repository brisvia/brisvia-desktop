// P0 journey #9 — Wrong password when revealing the phrase.
// On the real COMPILED app, verifies that if, when asking for the phrase (Settings -> Security -> Reveal),
// a WRONG password is entered, the backend REJECTS it: it shows an error message and does NOT reveal
// the 12 words (the phrase window does not open). This is the real protection of the backup.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';
const WRONG = 'wrong-password-9999';

describe('Journey 9 — wrong password', () => {
  it('rejects the wrong password and does not reveal the phrase', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);

    // Settings -> Security -> Reveal phrase, with the WRONG password.
    await (await $('.nav-btn[data-view="settings"]')).click();
    await (await $('#set-security')).click();
    await (await $('#modal-security')).waitForDisplayed({ timeout: 10000 });
    await (await $('#sec-reveal')).click();
    const revealModal = await $('#modal-reveal');
    await revealModal.waitForDisplayed({ timeout: 10000 });
    await (await $('#reveal-pass')).setValue(WRONG);
    await (await $('#reveal-go')).click();

    // The error message appears and the phrase window does NOT open.
    const msg = await $('#reveal-msg');
    await browser.waitUntil(async () => await msg.isDisplayed() && (await msg.getText()).trim().length > 0, {
      timeout: 10000, timeoutMsg: 'the wrong-password message did not appear',
    });
    expect(await (await $('#modal-seed')).isDisplayed()).toBe(false);
  });
});
