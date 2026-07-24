// P0 journey #15 — A live language change repaints DYNAMIC (JS-painted) text, not only static data-i18n.
//
// Why this exists: journey #11 only checks a STATIC label (the Wallet/Billetera tab, which carries data-i18n
// and is re-applied by I18N.setLang). It would NOT have caught the real bug: the power label, the balances and
// the auto-start summary are painted by JS with .textContent (no data-i18n), and the language handler did not
// repaint them, so they stayed in the previous language until an unrelated refresh. Fixed by reRenderForLanguage().
//
// We assert on #pow-val ("N de M hilos" / "N of M threads"), painted by refreshPowLabel(). The 1-second tick
// (refreshMine) only repaints it when the CPU core count CHANGES (app.js: `if (s.cores !== POW_CORES)`), never
// on a language change, so without the fix the stale language is PERMANENT and this test fails deterministically.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

async function setLang(lang) {
  await (await $('.nav-btn[data-view="settings"]')).click();
  await (await $('#set-language')).waitForDisplayed({ timeout: 10000 });
  await (await $(`#set-language .seg-btn[data-lang="${lang}"]`)).click();
}

describe('Journey 15 — language repaints dynamic (JS-painted) text', () => {
  it('the power label flips language live, not only the static labels', async () => {
    harness.fromEnv();
    await harness.onboardCreate(PASSWORD);

    // Mine tab: wait for the power label to carry a threads/hilos count (painted by JS once the miner status
    // brings the CPU core count; before that it shows just a percentage).
    await (await $('.nav-btn[data-view="mine"]')).click();
    await (await $('[data-testid="view-mine"]')).waitForDisplayed({ timeout: 10000 });
    const powVal = await $('#pow-val');
    await powVal.waitForDisplayed({ timeout: 10000 });
    await browser.waitUntil(async () => /hilos|threads/i.test(await powVal.getText()), {
      timeout: 20000, timeoutMsg: 'the power label never showed a threads/hilos count (POW_CORES not populated)',
    });

    // Switch to English: the JS-painted label must now read "threads". This is exactly what reRenderForLanguage
    // fixes — the 1s tick does not repaint #pow-val on its own, so a failure here is the real regression.
    await setLang('en');
    await browser.waitUntil(async () => {
      const t = await powVal.getText();
      return /threads/i.test(t) && !/hilos/i.test(t);
    }, { timeout: 8000, timeoutMsg: 'the JS-painted power label stayed in the old language after switching to English' });

    // Switch back to Spanish: the same dynamic label must read "hilos".
    await setLang('es');
    await browser.waitUntil(async () => /hilos/i.test(await powVal.getText()), {
      timeout: 8000, timeoutMsg: 'the JS-painted power label did not switch back to Spanish',
    });
  });
});
