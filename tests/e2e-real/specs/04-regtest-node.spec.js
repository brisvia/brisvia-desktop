// P0 journey #4 — Real regtest node: the app brings up the node and shows network and height.
// On the real COMPILED app (Rust backend + ephemeral regtest bitcoind), verifies that:
//   - the backend really brings up the node: the RPC responds (waitRpcUp);
//   - it is isolated regtest (getblockchaininfo -> chain == regtest);
//   - the app shows the node's real NETWORK and HEIGHT in the network panel (nr-network, nr-height).
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Journey 4 — regtest node', () => {
  it('brings up the node, the RPC responds and the app shows network and height', async () => {
    const run = harness.fromEnv();

    // 1) Enter the app (creating a wallet) to reach the network panel in the wallet view.
    await harness.onboardCreate(PASSWORD);

    // 2) The backend brought up the node: the RPC responds and it is isolated regtest.
    await harness.waitRpcUp(run.datadir, run.port, 60000);
    const chain = harness.rpc(run.datadir, run.port, ['getblockchaininfo']).stdout;
    expect(chain).toContain('"chain": "regtest"');

    // 3) The app shows the NETWORK (non-empty label) and the HEIGHT matching the node's.
    await (await $('.nav-btn[data-view="wallet"]')).click();
    const netEl = await $('[data-testid="nr-network"]');
    await browser.waitUntil(async () => {
      const t = (await netEl.getText()).trim();
      return t.length > 0 && t !== '—';
    }, { timeout: 20000, timeoutMsg: 'the network panel never showed the network' });

    const nodeHeight = harness.blockCount(run.datadir, run.port); // 0 at regtest genesis
    const heightEl = await $('[data-testid="nr-height"]');
    await browser.waitUntil(async () => {
      const t = (await heightEl.getText()).trim();
      return /^\d[\d.,]*$/.test(t) && parseInt(t.replace(/[.,]/g, ''), 10) === nodeHeight;
    }, { timeout: 20000, timeoutMsg: `the shown height did not match the node's (${nodeHeight})` });
  });
});
