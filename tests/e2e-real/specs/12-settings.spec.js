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

    // 2) Pool mining is OFF in 1.0 (POOL_ENABLED=false in the backend). The stratum engine is finished
    //    and tested against the real pool, but what a pool miner actually needs does not exist yet: seeing
    //    the connection, and the difference between a share found, a share submitted, and a share
    //    ACCEPTED. Shipping the engine with no honest way to see what it is doing is how someone ends up
    //    believing they mined for hours and were never paid.
    //
    //    The button must be genuinely DISABLED, not hidden: someone who came looking for pool mining
    //    deserves to know it is coming. And a control that still reacts would be worse -- it would promise
    //    something that does not run.
    const modePool = await $('#set-mining-mode .seg-btn[data-mode="pool"]');
    await browser.waitUntil(async () => !(await modePool.isEnabled()), {
      timeout: 5000,
      timeoutMsg: 'the pool mining button is enabled: 1.0 ships with solo mining only',
    });

    // 3) The reason, on screen. Disabled without saying why is a broken screen, not a safe one.
    await (await $('#pool-soon')).waitForDisplayed({ timeout: 5000 });

    // 4) Clicking it must do nothing: not activate, not open the pool row.
    await modePool.click().catch(() => {}); // a disabled button may refuse the click, which is fine
    await browser.pause(300);
    const activo = ((await modePool.getAttribute('class')) || '').includes('active');
    if (activo) throw new Error('pool mode activated even though it is disabled');
    if (await (await $('#pool-info-row')).isDisplayed()) {
      throw new Error('the pool row opened while pool mode is disabled');
    }

    // 5) Solo mining: still selectable, and the only thing 1.0 promises.
    const modeSolo = await $('#set-mining-mode .seg-btn[data-mode="solo"]');
    if (!(await modeSolo.isEnabled())) throw new Error('solo mode ended up disabled');
    await modeSolo.click();
    await browser.waitUntil(async () => ((await modeSolo.getAttribute('class')) || '').includes('active'), {
      timeout: 5000, timeoutMsg: 'solo mode did not become active',
    });
  });
});
