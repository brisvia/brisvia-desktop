// P0 journey #2 — Create wallet (real onboarding flow).
// On the real COMPILED app (Rust backend generates the 12 words and encrypts the wallet), verifies that:
//   - onboarding walks welcome -> choose -> password -> seed -> verification;
//   - the backend generates 12 real words and shows them;
//   - the backup verification (pick the requested words in order) confirms the real backup;
//   - on finishing, the app leaves onboarding and shows the wallet with the version loaded from the backend.
// This is "real backend without a node": creating the wallet does not depend on the node being up.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234'; // >= 8 characters, same in both fields

describe('Journey 2 — create wallet', () => {
  it('generates 12 real words, verifies the backup and enters the wallet', async () => {
    harness.fromEnv();

    // 1) Welcome: the app starts without a wallet and shows onboarding.
    const welcome = await $('[data-testid="onb-welcome"]');
    await welcome.waitForDisplayed({ timeout: 60000 });

    // 2) Go through the 3 welcome slides until reaching "create or import".
    const choose = await $('[data-testid="onb-choose"]');
    const next = await $('[data-testid="onb-next"]');
    for (let i = 0; i < 5 && !(await choose.isDisplayed()); i++) {
      await next.waitForClickable({ timeout: 10000 });
      await next.click();
      await browser.pause(150); // let the slide re-render; the real cutoff is isDisplayed()
    }
    await choose.waitForDisplayed({ timeout: 10000 });

    // 3) Choose "create wallet" -> password step.
    const create = await $('[data-testid="onb-create"]');
    await create.waitForClickable({ timeout: 10000 });
    await create.click();

    const pass = await $('[data-testid="onb-pass"]');
    await pass.waitForDisplayed({ timeout: 10000 });
    const p1 = await $('[data-testid="pass-1"]');
    const p2 = await $('[data-testid="pass-2"]');
    await p1.setValue(PASSWORD);
    await p2.setValue(PASSWORD);
    const passNext = await $('[data-testid="pass-next"]');
    await passNext.click();

    // 4) The backend generated the 12 words and shows them. We read them to verify the backup.
    const seedStep = await $('[data-testid="onb-seed"]');
    await seedStep.waitForDisplayed({ timeout: 30000 });
    const seedGrid = await $('[data-testid="seed-grid"]');
    await browser.waitUntil(async () => (await seedGrid.$$('li')).length === 12, {
      timeout: 30000,
      timeoutMsg: 'the backend did not return 12 words (did wallet.create fail?)',
    });
    const seedItems = await seedGrid.$$('li');
    const seed = [];
    for (const li of seedItems) seed.push((await li.getText()).trim());
    expect(seed.filter(Boolean).length).toBe(12);

    // 5) Confirm they were written down and advance to verification.
    const ack = await $('[data-testid="seed-ack"]');
    await ack.click();
    const seedNext = await $('[data-testid="seed-next"]');
    await browser.waitUntil(async () => await seedNext.isEnabled(), {
      timeout: 5000, timeoutMsg: 'the seed continue button did not enable after ticking the check',
    });
    await seedNext.click();

    // 6) Backup verification: the app asks for 3 words by their position. We read the requested positions
    //    and pick the correct word from the bank, in the requested order (ascending positions).
    const verifyStep = await $('[data-testid="onb-verify"]');
    await verifyStep.waitForDisplayed({ timeout: 10000 });
    const slotEls = await $$('[data-testid="verify-slots"] .slot');
    const positions = [];
    for (const s of slotEls) {
      const n = parseInt((await s.$('.slot-n').getText()).trim(), 10);
      positions.push(n); // 1-indexed, exactly as the UI shows it
    }
    expect(positions.length).toBe(3);

    for (const pos of positions) {
      const word = seed[pos - 1];
      // Pick the bank chip with that word that has not been used yet.
      const chips = await $$('[data-testid="verify-bank"] .chip');
      let clicked = false;
      for (const chip of chips) {
        const cls = (await chip.getAttribute('class')) || '';
        if (cls.includes('used')) continue;
        if ((await chip.getText()).trim() === word) {
          await chip.click();
          clicked = true;
          break;
        }
      }
      expect(clicked).toBe(true);
    }

    // 7) Verification passed (the app confirms the backup in the backend and leaves onboarding).
    const setup = await $('#setup');
    await browser.waitUntil(async () => !(await setup.isDisplayed()), {
      timeout: 20000,
      timeoutMsg: 'onboarding did not close after verifying the backup (did verification fail?)',
    });

    // 8) We are in the wallet and the backend responds: the wallet view shows and the version is loaded.
    const walletView = await $('[data-testid="view-wallet"]');
    await walletView.waitForDisplayed({ timeout: 15000 });
    const ver = await $('[data-testid="ver-chip"]');
    expect((await ver.getText()).trim()).toMatch(/^v\d/);
  });
});
