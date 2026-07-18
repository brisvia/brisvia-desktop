// P0 flow #1 — First launch.
// Verifies, on the real COMPILED app (Rust backend + ephemeral regtest node), that:
//   - the app starts and the frontend loads its resources (version chip filled from the backend's app_version);
//   - the backend responds and actually brings up the node (the regtest bitcoind RPC becomes available);
//   - the app does not end in a fatal error: on detecting there is no wallet, it shows onboarding;
//   - the node is really regtest and isolated (the e2e redirection works end to end).
// (That no orphan processes remain is validated at session teardown, in the harness afterSession.)
'use strict';

const harness = require('../helpers/harness');

describe('Flow 1 — first launch', () => {
  it('starts, the backend responds, brings up the regtest node and shows onboarding without a fatal error', async () => {
    const run = harness.fromEnv();

    // 1) The frontend loaded and talked to the backend: the version chip is filled from app_version.
    const ver = await $('[data-testid="ver-chip"]');
    await ver.waitForExist({ timeout: 30000 });
    await browser.waitUntil(async () => (await ver.getText()).trim().length > 0, {
      timeout: 30000,
      timeoutMsg: 'the version chip never filled (did the backend fail to respond to app_version?)',
    });
    expect((await ver.getText()).trim()).toMatch(/^v\d/);

    // 2) The backend brought up the node: the regtest bitcoind RPC responds.
    await harness.waitRpcUp(run.datadir, run.port, 60000);

    // 3) It is really regtest (the e2e redirection reached the node).
    const chain = harness.rpc(run.datadir, run.port, ['getblockchaininfo']).stdout;
    expect(chain).toContain('"chain": "regtest"');

    // 4) No wallet on disk -> the app decides to show onboarding, a sign of a healthy startup.
    const welcome = await $('[data-testid="onb-welcome"]');
    await welcome.waitForDisplayed({ timeout: 60000 });
    expect(await welcome.isDisplayed()).toBe(true);
  });
});
