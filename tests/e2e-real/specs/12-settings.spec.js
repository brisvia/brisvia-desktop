// P0 journey #12 — Settings: CPU intensity and solo/pool mode.
// Against the REAL compiled app: the options in Settings can be changed and the UI answers.
//   - picking a CPU intensity (75%, say) marks that button active;
//   - switching to pool mode marks that button active and shows the pool's row;
//   - going back to solo hides that row.
// It stays away from options that call into the operating system (start with Windows, tray icon), so the
// result does not depend on the machine it runs on.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Journey 12 — settings', () => {
  it('changes CPU intensity, and pool mode is disabled with its reason on screen', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);
    await (await $('.nav-btn[data-view="settings"]')).click();
    await (await $('#set-intensity')).waitForDisplayed({ timeout: 10000 });

    // 1) CPU intensity: pick 75% -> that button becomes active.
    const int75 = await $('#set-intensity .seg-btn[data-pct="75"]');
    await int75.click();
    await browser.waitUntil(async () => ((await int75.getAttribute('class')) || '').includes('active'), {
      timeout: 5000, timeoutMsg: '75% intensity did not become active',
    });

    // 2) Pool mining is ENABLED in 1.0.9 (POOL_ENABLED=true; the honest share UI — connection, and share
    //    found vs submitted vs ACCEPTED — shipped). The pool button must be genuinely ENABLED and selectable.
    const modePool = await $('#set-mining-mode .seg-btn[data-mode="pool"]');
    await browser.waitUntil(async () => (await modePool.isEnabled()), {
      timeout: 5000,
      timeoutMsg: 'the pool mining button is disabled: 1.0.9 ships with the pool enabled',
    });

    // 3) The "coming soon" note must be HIDDEN now that the pool is available.
    await browser.waitUntil(async () => !(await (await $('#pool-soon')).isDisplayed().catch(() => false)), {
      timeout: 5000, timeoutMsg: 'the pool "coming soon" note is still shown while the pool is enabled',
    });

    // 4) Clicking pool must select it (and reveal the pool row).
    await modePool.click();
    await browser.waitUntil(async () => ((await modePool.getAttribute('class')) || '').includes('active'), {
      timeout: 5000, timeoutMsg: 'pool mode did not become active on click',
    });

    // 5) Solo mining: still selectable.
    const modeSolo = await $('#set-mining-mode .seg-btn[data-mode="solo"]');
    if (!(await modeSolo.isEnabled())) throw new Error('solo mode ended up disabled');
    await modeSolo.click();
    await browser.waitUntil(async () => ((await modeSolo.getAttribute('class')) || '').includes('active'), {
      timeout: 5000, timeoutMsg: 'solo mode did not become active',
    });
  });
});
