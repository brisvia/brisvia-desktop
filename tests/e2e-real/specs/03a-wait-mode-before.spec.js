// P0 flow #3a — Wait mode BEFORE launch (real-network build + clock frozen before 2026-08-01).
// On the real COMPILED REAL-NETWORK app (mainnet-e2e binary) with the clock set BEFORE MAINNET_START,
// verifies that:
//   - the wallet works (you can create/enter it);
//   - mining is ON HOLD: the status reads "Próximamente" (wait.badge) and the mine button is disabled;
//   - the network panel does NOT say "Sincronizando", but "En espera de lanzamiento" (fix for the known pending item).
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Flow 3a — wait mode (before launch)', () => {
  it('shows "on hold", the mine button disabled and the network panel without "Sincronizando"', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);

    // Expected texts based on the app's real language (locale-proof). We compare in lowercase because
    // the badge is shown in UPPERCASE via CSS (text-transform) and getText() respects that style.
    const waitBadge = (await browser.execute(() => window.I18N.t('wait.badge'))).toLowerCase();
    const waitNet = (await browser.execute(() => window.I18N.t('wait.net'))).toLowerCase();

    // Mine view: "on hold" status + disabled button (we give pollNet time to detect the real network).
    await (await $('.nav-btn[data-view="mine"]')).click();
    const badge = await $('[data-testid="state-badge"]');
    await browser.waitUntil(async () => (await badge.getText()).trim().toLowerCase() === waitBadge, {
      timeout: 20000, timeoutMsg: `the status did not stay on hold ("${waitBadge}")`,
    });
    expect(await (await $('[data-testid="mine-toggle"]')).isEnabled()).toBe(false);

    // Network panel (wallet view): in wait mode it reads "En espera de lanzamiento", never "Sincronizando".
    await (await $('.nav-btn[data-view="wallet"]')).click();
    const status = await $('[data-testid="nr-status"]');
    await browser.waitUntil(async () => (await status.getText()).trim().toLowerCase() === waitNet, {
      timeout: 20000, timeoutMsg: `the network panel did not show "${waitNet}" in wait mode`,
    });
  });
});
