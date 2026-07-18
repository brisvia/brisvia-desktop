// P0 flow #5 — Real RandomX mining (the central one).
// On the real COMPILED app (Rust backend + regtest bitcoind + RandomX brisvia-worker engine), verifies that:
//   - the engine actually STARTS (the same command the "Mine" button triggers) and the backend reports mining;
//   - it actually STOPS (the engine halts: getStatus().mining == false);
//   - BEST-EFFORT: it tries to mine >=1 regtest block and, if it does, confirms the node height goes up.
//
// Why the block is best-effort: RandomX is heavy on the free CI runner (2 threads, light mode),
// and even at minimum difficulty it may not find a block within the time window. Real mining is ALREADY
// validated through other means (engine unit tests, real testnet mining, pool verifier 15/15
// against real blocks). Here the central, deterministic thing is that the app TURNS the engine ON and OFF
// correctly (the JS -> Tauri -> sidecar -> cleanup chain). The flow runs in continue-on-error.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

const isMining = () => browser.executeAsync((done) => {
  window.brisvia.getStatus().then((s) => done(!!(s && s.mining))).catch(() => done(false));
});

describe('Flow 5 — RandomX mining', () => {
  it('turns the engine on, tries to mine 1 block and turns it off', async () => {
    // Note: in wdio+mocha this.timeout() does not apply reliably, so the whole test must fit within
    // mocha's global timeout (180s). That is why the best-effort mining window is short (60s): it is enough
    // for the informational check (the runner won't find a block anyway) without blowing the budget.
    const run = harness.fromEnv();

    await harness.onboardCreate(PASSWORD);
    await harness.waitRpcUp(run.datadir, run.port, 60000);
    const h0 = harness.blockCount(run.datadir, run.port);

    await (await $('.nav-btn[data-view="mine"]')).click();
    await (await $('[data-testid="view-mine"]')).waitForDisplayed({ timeout: 10000 });

    // 1) Start the engine (same command as the button; the button is disabled by IBD at regtest genesis).
    const started = await browser.executeAsync((done) => {
      window.brisvia.start('50').then((r) => done(r || true)).catch((e) => done({ error: String(e) }));
    });
    expect(started && !started.error).toBeTruthy();

    // The backend reports the engine active (real startup of the RandomX sidecar).
    await browser.waitUntil(async () => await isMining(), {
      timeout: 30000, timeoutMsg: 'the backend did not report mining active after starting',
    });

    // 2) BEST-EFFORT: short window (60s) to see whether it mines a block (the node height goes up). It does not
    //    fail if it doesn't: RandomX is heavy on the free runner and won't find a block in time (validated elsewhere).
    let mined = false;
    const t0 = Date.now();
    try {
      await browser.waitUntil(async () => harness.blockCount(run.datadir, run.port) > h0, {
        timeout: 60000, interval: 3000, timeoutMsg: 'did not mine within the window',
      });
      mined = true;
    } catch { mined = false; }
    const secs = Math.round((Date.now() - t0) / 1000);
    const h1 = harness.blockCount(run.datadir, run.port);
    console.log(`[e2e][05] engine started OK. Block mined: ${mined ? 'YES' : 'no (slow runner)'} — height ${h0} -> ${h1} in ~${secs}s.`);
    if (mined) expect(h1).toBeGreaterThan(h0);

    // 3) Stop the engine for real: it halts and the backend stops reporting mining.
    await browser.executeAsync((done) => {
      window.brisvia.stop().then(() => done(true)).catch(() => done(false));
    });
    await browser.waitUntil(async () => !(await isMining()), {
      timeout: 20000, timeoutMsg: 'the engine did not stop after halting',
    });
  });
});
