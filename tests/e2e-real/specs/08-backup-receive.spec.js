// P0 journey #8 — Backup (see the 12 words again) + Receive (address).
// On the real COMPILED app (real Rust backend, without depending on the node), verifies that:
//   - a wallet is created (the backend generates 12 real words) and entered;
//   - from Settings -> Security the 12 words can be REVEALED by asking for the password,
//     and they are EXACTLY the same ones generated when it was created (real backup, not cosmetic);
//   - Receive shows a real address from the wallet.
// This is "real backend without a node": create/reveal/address do not depend on the node being up.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Journey 8 — backup and receive', () => {
  it('reveals the same 12 words with the password and shows an address to receive', async () => {
    harness.fromEnv();

    const seed = await harness.onboardCreate(PASSWORD);

    // 1) Go to Settings -> open Security -> Reveal phrase.
    await (await $('.nav-btn[data-view="settings"]')).click();
    const openSecurity = await $('#set-security');
    await openSecurity.waitForClickable({ timeout: 10000 });
    await openSecurity.click();
    await (await $('#modal-security')).waitForDisplayed({ timeout: 10000 });
    await (await $('#sec-reveal')).click();

    // 2) Ask for the phrase with the correct password.
    const revealModal = await $('#modal-reveal');
    await revealModal.waitForDisplayed({ timeout: 10000 });
    await (await $('#reveal-pass')).setValue(PASSWORD);
    await (await $('#reveal-go')).click();

    // 3) The 12 words are shown and match EXACTLY the ones created.
    const seedModal = await $('#modal-seed');
    await seedModal.waitForDisplayed({ timeout: 15000 });
    const grid = await $('#seed-grid-view');
    await browser.waitUntil(async () => (await grid.$$('li')).length === 12, {
      timeout: 10000, timeoutMsg: 'the revealed phrase did not show 12 words (password rejected?)',
    });
    const revealed = [];
    for (const li of await grid.$$('li')) revealed.push((await li.getText()).trim().replace(/^\d+[.)]?\s*/, ''));
    expect(revealed.join(' ')).toBe(seed.join(' '));

    // Close the phrase modal.
    await (await seedModal.$('[data-close]')).click();

    // 4) Receive: a real address from the wallet is shown.
    const addr = await harness.readReceiveAddress();
    expect(addr.length).toBeGreaterThan(10);
  });
});
