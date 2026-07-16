// P0 journey #7 — Restore wallet from 12 words (the real "switching PCs" flow).
// On the real COMPILED app (real Rust backend, without depending on the node), verifies that:
//   - onboarding lets you choose "import" and shows the grid of 12 boxes;
//   - you can type 12 valid words (standard BIP39 phrase) and advance;
//   - a new password is requested to encrypt the restored wallet;
//   - the backend really restores (wallet.restore) and leaves onboarding showing the wallet with an address.
// This is "real backend without a node": restoring does not depend on the node being up.
'use strict';

const harness = require('../helpers/harness');

// Canonical BIP39 test phrase (zero entropy, valid checksum). The backend uses the `bip39` crate
// with the standard English wordlist, so it accepts it. It is public and for testing: it never holds real funds.
const SEED = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const PASSWORD = 'brisvia-e2e-1234';

describe('Journey 7 — restore wallet', () => {
  it('imports 12 valid words, asks for a password and enters the wallet with an address', async () => {
    harness.fromEnv();

    // 1) Welcome -> choose "import" -> the grid of 12 boxes appears.
    await harness.skipWelcome();
    const importBtn = await $('#btn-import');
    await importBtn.waitForClickable({ timeout: 10000 });
    await importBtn.click();

    const importGrid = await $('#import-grid');
    await importGrid.waitForDisplayed({ timeout: 10000 });
    await browser.waitUntil(async () => (await importGrid.$$('input')).length === 12, {
      timeout: 10000, timeoutMsg: 'the import grid did not show 12 boxes',
    });

    // 2) Type the 12 words (one per box, in order).
    const words = SEED.split(' ');
    const inputs = await importGrid.$$('input');
    for (let i = 0; i < 12; i++) await inputs[i].setValue(words[i]);

    // 3) Confirm the phrase -> password step.
    await (await $('#import-ok')).click();
    const pass = await $('[data-testid="onb-pass"]');
    await pass.waitForDisplayed({ timeout: 10000 });
    await (await $('[data-testid="pass-1"]')).setValue(PASSWORD);
    await (await $('[data-testid="pass-2"]')).setValue(PASSWORD);
    await (await $('[data-testid="pass-next"]')).click();

    // 4) The backend restores and leaves onboarding: the wallet view appears and the version is loaded.
    const setup = await $('#setup');
    await browser.waitUntil(async () => !(await setup.isDisplayed()), {
      timeout: 30000, timeoutMsg: 'onboarding did not close after restoring (did wallet.restore fail?)',
    });
    const walletView = await $('[data-testid="view-wallet"]');
    await walletView.waitForDisplayed({ timeout: 15000 });
    expect((await (await $('[data-testid="ver-chip"]')).getText()).trim()).toMatch(/^v\d/);

    // 5) The restored wallet gives a real address to receive.
    const addr = await harness.readReceiveAddress();
    expect(addr.length).toBeGreaterThan(10);
  });
});
