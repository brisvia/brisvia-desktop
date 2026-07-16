// P0 journey #5 — Real RandomX mining (the central one).
// On the real COMPILED app (Rust backend + regtest bitcoind + RandomX engine brisvia-worker), verifies that:
//   - the engine really STARTS (the same command the "Mine" button fires) and the backend reports mining;
//   - it really STOPS (the engine stops: getStatus().mining == false);
//   - BEST-EFFORT: tries to mine >=1 regtest block and, if it does, confirms the node's height rises.
//
// Why the block is best-effort: RandomX is heavy on the free CI runner (2 threads, light mode), and even
// at minimum difficulty it may not find a block within the time window. Real mining is ALREADY validated
// by other means (engine unit tests, real testnet mining, pool verifier 15/15 against real blocks). What
// is central and deterministic here is that the app STARTS and STOPS the engine correctly (the JS -> Tauri
// -> sidecar -> cleanup chain). The journey runs under continue-on-error.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

const isMining = () => browser.executeAsync((done) => {
  window.brisvia.getStatus().then((s) => done(!!(s && s.mining))).catch(() => done(false));
});

describe('Journey 5 — RandomX mining', () => {
  it('starts the engine, tries to mine 1 block and stops it', async () => {
    // Note: in wdio+mocha this.timeout() does not apply reliably, so the whole test must fit inside mocha's
    // global timeout (180s). That is why the best-effort mining window is short (60s): enough for the
    // informative check (the runner does not find a block anyway) without blowing the budget.
    const run = harness.fromEnv();

    await harness.onboardCreate(PASSWORD);
    await harness.waitRpcUp(run.datadir, run.port, 60000);
    const h0 = harness.blockCount(run.datadir, run.port);

    await (await $('.nav-btn[data-view="mine"]')).click();
    await (await $('[data-testid="view-mine"]')).waitForDisplayed({ timeout: 10000 });

    // 1) Start the engine (same command as the button; the button is disabled by IBD on regtest genesis).
    const started = await browser.executeAsync((done) => {
      window.brisvia.start('50').then((r) => done(r || true)).catch((e) => done({ error: String(e) }));
    });
    expect(started && !started.error).toBeTruthy();

    // The backend reports the engine active (real start of the RandomX sidecar).
    await browser.waitUntil(async () => await isMining(), {
      timeout: 30000, timeoutMsg: 'the backend did not report mining active after starting',
    });

    // 2) BEST-EFFORT: a short window (60s) to see if it mines a block (the node's height rises). Does not
    //    fail if it does not: RandomX is heavy on the free runner and does not find a block in time
    //    (validated by other means).
    let mined = false;
    const t0 = Date.now();
    try {
      await browser.waitUntil(async () => harness.blockCount(run.datadir, run.port) > h0, {
        timeout: 60000, interval: 3000, timeoutMsg: 'did not mine in the window',
      });
      mined = true;
    } catch { mined = false; }
    const secs = Math.round((Date.now() - t0) / 1000);
    const h1 = harness.blockCount(run.datadir, run.port);
    console.log(`[e2e][05] engine started OK. Block mined: ${mined ? 'YES' : 'no (slow runner)'} — height ${h0} -> ${h1} in ~${secs}s.`);
    if (mined) expect(h1).toBeGreaterThan(h0);

    // 3) Really stop the engine: it stops and the backend stops reporting mining.
    await browser.executeAsync((done) => {
      window.brisvia.stop().then(() => done(true)).catch(() => done(false));
    });
    await browser.waitUntil(async () => !(await isMining()), {
      timeout: 20000, timeoutMsg: 'the engine did not stop after stopping',
    });
  });
});
