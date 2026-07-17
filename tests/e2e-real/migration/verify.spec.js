// Migration test, step 2 (runs on the INSTALLED 1.0.8 after installing it over 1.0.7, same datadir).
// Proves the update kept the wallet: the app opens STRAIGHT to the wallet (no onboarding, so no new
// wallet was silently created), and the SAME password decrypts the seed (revealing the phrase returns
// twelve words). The byte-for-byte survival of wallet_seed.enc and the no-reindex check are asserted by
// the workflow around this spec; here we prove the wallet is functionally the same one, unlocked by the
// same password, through the real installed app. We also confirm the shipped binary runs with pool mining
// OFF (POOL_ENABLED=false) at RUNTIME, not just in source.
'use strict';

const PASSWORD = 'brisvia-e2e-1234';

describe('Migration verify — the wallet survived the update (installed 1.0.8)', () => {
  it('opens straight to the wallet (no new wallet) and unlocks with the same password', async () => {
    const walletView = await $('[data-testid="view-wallet"]');
    const welcome = await $('[data-testid="onb-welcome"]');
    await browser.waitUntil(async () => (await walletView.isDisplayed()) || (await welcome.isDisplayed()), {
      timeout: 60000, timeoutMsg: 'after the update the app showed neither the wallet nor onboarding',
    });
    // If onboarding reappears, the update lost the wallet (or created a new one): a hard fail.
    expect(await welcome.isDisplayed()).toBe(false);
    await walletView.waitForDisplayed({ timeout: 15000 });

    // The shipped 1.0.8 binary must run with pool mining OFF. Read it from the REAL app's status (not the
    // source): POOL_ENABLED is a hardcoded const, so the running binary reports poolEnabled=false. getStatus()
    // is async, so it MUST go through executeAsync with a done callback (plain execute does not await the
    // promise on classic WebDriver — it returns undefined). A missing field surfaces as a non-false value.
    const poolEnabled = await browser.executeAsync((done) => {
      window.brisvia.getStatus()
        .then((s) => done(s && typeof s.poolEnabled === 'boolean' ? s.poolEnabled : 'status-missing-poolEnabled'))
        .catch((e) => done('getStatus-failed:' + String(e)));
    });
    expect(poolEnabled).toBe(false);

    // The same password must decrypt the seed. Reveal shows twelve words only if wallet_seed.enc opened
    // with this password — proving the encrypted seed survived and stays readable by 1.0.6.
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
      timeout: 10000, timeoutMsg: 'the previous password did not decrypt the seed after the update',
    });
  });
});
