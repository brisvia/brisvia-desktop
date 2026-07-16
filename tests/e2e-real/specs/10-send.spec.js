// P0 journey #10 — Send: modal, "use max" and validations.
// On the real COMPILED app, verifies the send modal WITHOUT actually sending money:
//   - it opens the modal;
//   - an empty/invalid address gives an error ("invalid address");
//   - with a valid address but amount 0/negative it gives an error ("invalid amount");
//   - "use max" fills the amount with the available balance.
// It does not run a real send (the new wallet has no funds): it only validates the input UX.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Journey 10 — send (validations)', () => {
  it('validates address and amount and fills the max', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);

    // Open the send modal from the wallet.
    await (await $('.nav-btn[data-view="wallet"]')).click();
    await (await $('#act-send')).click();
    const sendModal = await $('#modal-send');
    await sendModal.waitForDisplayed({ timeout: 10000 });

    const addr = await $('#send-addr');
    const amount = await $('#send-amount');
    const go = await $('#send-go');
    const msg = await $('#send-msg');

    // 1) Invalid address -> address error.
    await addr.setValue('not-an-address');
    await amount.setValue('1');
    await go.click();
    await browser.waitUntil(async () => await msg.isDisplayed() && (await msg.getText()).trim().length > 0, {
      timeout: 8000, timeoutMsg: 'it did not reject the invalid address',
    });
    const errAddr = (await msg.getText()).trim();
    expect(errAddr.length).toBeGreaterThan(0);

    // 2) Address with valid format (contains "brv" and long enough) but amount 0 -> amount error.
    await addr.setValue('brv1qexampleexampleexampleexample00');
    await amount.setValue('0');
    await go.click();
    await browser.waitUntil(async () => {
      const t = (await msg.getText()).trim();
      return await msg.isDisplayed() && t.length > 0 && t !== errAddr;
    }, { timeout: 8000, timeoutMsg: 'it did not reject the invalid amount' });

    // 3) "Use max" fills the amount with the available balance (0 in a new wallet, but it fills the field).
    await (await $('#send-max')).click();
    await browser.waitUntil(async () => (await amount.getValue()).trim().length > 0, {
      timeout: 5000, timeoutMsg: 'the use-max button did not fill the amount',
    });

    // Close the modal.
    await (await sendModal.$('[data-close]')).click();
  });
});
