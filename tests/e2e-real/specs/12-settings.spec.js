// P0 flow #12 — Configuration: CPU intensity and solo/pool mode.
// On the real COMPILED app, verifies that the options in Settings can be changed and the UI responds:
//   - choosing a CPU intensity (e.g. 75%) marks that button as active;
//   - switching to "pool" mode marks that button active and shows the row with the pool details;
//   - going back to "solo" hides that row.
// It does not touch options that call the operating system (start with Windows / tray) to avoid depending on the environment.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Flow 12 — configuration', () => {
  it('changes the CPU intensity, and pool mode is disabled with its reason visible', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);
    await (await $('.nav-btn[data-view="settings"]')).click();
    await (await $('#set-intensity')).waitForDisplayed({ timeout: 10000 });

    // 1) CPU intensity: choose 75% -> that button stays active.
    const int75 = await $('#set-intensity .seg-btn[data-pct="75"]');
    await int75.click();
    await browser.waitUntil(async () => ((await int75.getAttribute('class')) || '').includes('active'), {
      timeout: 5000, timeoutMsg: 'the 75% intensity did not stay active',
    });

    // 2) Pool mining is OFF in 1.0 (POOL_ENABLED=false in the backend). The stratum engine is
    //    finished and tested against the real pool, but what a pool user needs does not exist yet:
    //    seeing the connection, and the difference between a share found, sent and ACCEPTED. Shipping the engine
    //    without an honest way to see what it does is how someone ends up believing they mined for hours and got paid nothing.
    //
    //    The button must be truly DISABLED, not hidden: whoever came looking for pool deserves
    //    to know it is coming. And a control that reacts anyway would be worse: it would promise something that does not run.
    const modePool = await $('#set-mining-mode .seg-btn[data-mode="pool"]');
    await browser.waitUntil(async () => !(await modePool.isEnabled()), {
      timeout: 5000,
      timeoutMsg: 'the pool mining button is enabled: 1.0 ships with solo mining only',
    });

    // 3) The reason, visible. Being disabled without explaining why is a broken screen.
    await (await $('#pool-soon')).waitForDisplayed({ timeout: 5000 });

    // 4) Clicking it must do NOTHING: neither activate, nor open the pool row.
    await modePool.click().catch(() => {}); // a disabled button may reject the click: it does not matter
    await browser.pause(300);
    const activo = ((await modePool.getAttribute('class')) || '').includes('active');
    if (activo) throw new Error('pool mode activated despite being disabled');
    if (await (await $('#pool-info-row')).isDisplayed()) {
      throw new Error('the pool row opened with pool mode disabled');
    }

    // 5) Solo mining: still selectable, and it is the only thing 1.0 promises.
    const modeSolo = await $('#set-mining-mode .seg-btn[data-mode="solo"]');
    if (!(await modeSolo.isEnabled())) throw new Error('solo mode ended up disabled');
    await modeSolo.click();
    await browser.waitUntil(async () => ((await modeSolo.getAttribute('class')) || '').includes('active'), {
      timeout: 5000, timeoutMsg: 'solo mode did not stay active',
    });
  });
});
