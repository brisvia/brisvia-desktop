// P0 flow #3b — Wait mode AFTER launch (real-network build + clock frozen after 2026-08-01).
// On the real COMPILED REAL-NETWORK app (mainnet-e2e binary) with the clock set AFTER MAINNET_START,
// verifies that the app is NO LONGER on hold: the status stops reading "Próximamente" (crossing the date
// enables mining on its own, with no user action). The button may stay disabled for a moment due to
// the node's real sync, but the status must NOT be "on hold".
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Flow 3b — wait mode (after launch)', () => {
  it('is no longer on hold: the status stops being "Próximamente" once the date passes', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);

    // In lowercase: the badge is shown in UPPERCASE via CSS and getText() respects that style.
    const waitBadge = (await browser.execute(() => window.I18N.t('wait.badge'))).toLowerCase();
    const waitTitle = (await browser.execute(() => window.I18N.t('wait.title'))).toLowerCase();

    await (await $('.nav-btn[data-view="mine"]')).click();
    const badge = await $('[data-testid="state-badge"]');
    const hero = await $('[data-testid="hero-title"]');

    // We give pollNet room to detect the real network and recompute the mode. The status must NOT stay "on hold".
    await browser.waitUntil(async () => {
      const b = (await badge.getText()).trim().toLowerCase();
      const h = (await hero.getText()).trim().toLowerCase();
      return b.length > 0 && b !== waitBadge && h !== waitTitle;
    }, { timeout: 30000, timeoutMsg: 'after the date passed, the app kept showing the "on hold" status' });

    expect((await badge.getText()).trim().toLowerCase()).not.toBe(waitBadge);
  });
});
