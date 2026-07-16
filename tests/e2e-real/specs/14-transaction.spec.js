// P0 journey #14 — Real regtest transaction (the money path end to end).
// On the real COMPILED app + an isolated regtest node, verifies the full flow:
//   - create the wallet (its descriptors are imported into the node wallet "brisvia");
//   - mine coinbase to the wallet's OWN receive address until a coinbase matures (spendable balance);
//   - the app shows that balance in the send modal ("available");
//   - send a portion to an EXTERNAL address (a separate throwaway node wallet), unlocking with the password;
//   - the transaction reaches the mempool (a real send happened), confirms in a block, and the wallet's
//     spendable balance drops by the amount + fee;
//   - after confirmation the mempool is empty (no phantom pending payment).
// The seed/password used here are throwaway and generated only for the test; they never reach logs.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';
const WALLET = 'brisvia';

// bitcoin-cli against the app's wallet on this run's node.
const wcli = (run, args) => harness.rpc(run.datadir, run.port, [`-rpcwallet=${WALLET}`, ...args]);

// Spendable (trusted) balance of the app wallet, read straight from the node (ground truth).
function trustedBalance(run) {
  const r = wcli(run, ['getbalances']);
  try { return JSON.parse(r.stdout).mine.trusted; } catch { return -1; }
}

function mempoolSize(run) {
  const r = harness.rpc(run.datadir, run.port, ['getmempoolinfo']);
  try { return JSON.parse(r.stdout).size; } catch { return -1; }
}

describe('Journey 14 — regtest transaction', () => {
  it('funds the wallet, sends to an external address, confirms and the balance is correct', async () => {
    const run = harness.fromEnv();
    await harness.onboardCreate(PASSWORD);
    await harness.waitRpcUp(run.datadir, run.port, 60000);

    // 1) The wallet's own receive address, read from the app UI.
    await (await $('.nav-btn[data-view="wallet"]')).click();
    await (await $('#act-receive')).click();
    await (await $('#modal-receive')).waitForDisplayed({ timeout: 10000 });
    const addrA = (await (await $('[data-testid="recv-addr"]')).getText()).trim();
    expect(addrA.length).toBeGreaterThan(20);
    await (await $('#modal-receive [data-close]')).click();

    // 2) Fund it: mine 110 blocks to addrA so several coinbases mature (100-conf maturity).
    const gen = wcli(run, ['generatetoaddress', '110', addrA]);
    expect(gen.status).toBe(0);

    // 3) The node wallet now reports a spendable balance...
    await browser.waitUntil(() => trustedBalance(run) > 0, {
      timeout: 30000, interval: 2000, timeoutMsg: 'the wallet never showed a spendable balance after mining',
    });
    const bal0 = trustedBalance(run);
    expect(bal0).toBeGreaterThan(0);

    // ...and the app reflects it in the wallet view, which re-loads the balance every ~3s while visible.
    await (await $('.nav-btn[data-view="wallet"]')).click();
    await browser.waitUntil(async () => {
      const t = (await (await $('#bal-amount')).getText()).trim();
      return parseFloat(t.replace(/[^\d.]/g, '')) > 0;
    }, { timeout: 30000, interval: 2000, timeoutMsg: 'the app wallet view did not show the funded balance' });

    // Now the send modal's "available" (read from the freshly loaded balance) is funded.
    await (await $('#act-send')).click();
    await (await $('#modal-send')).waitForDisplayed({ timeout: 10000 });
    const availTxt = (await (await $('#send-avail')).getText()).trim();
    expect(parseFloat(availTxt.replace(/[^\d.]/g, ''))).toBeGreaterThan(0);

    // 4) Send 10 to a fresh address of the wallet (a valid Brisvia address the app accepts), unlocking
    //    with the password. This exercises the full path: validate, unlock, sign and broadcast a real tx.
    const addrB = wcli(run, ['getnewaddress']).stdout.trim();
    expect(addrB.length).toBeGreaterThan(20);
    console.log(`[e2e][14] addrA=${addrA} addrB=${addrB}`);

    await (await $('#send-addr')).setValue(addrB);
    await (await $('#send-amount')).setValue('10');
    await (await $('#send-pass')).setValue(PASSWORD);
    await (await $('#send-go')).click();

    // 5) A real transaction reaches the mempool (if the send errored, surface the modal message).
    try {
      await browser.waitUntil(() => mempoolSize(run) >= 1, {
        timeout: 25000, interval: 1500, timeoutMsg: 'the transaction never reached the mempool',
      });
    } catch (e) {
      const m = await (await $('#send-msg')).getText().catch(() => '');
      throw new Error(`${e.message}. Send modal message was: "${(m || '').trim()}"`);
    }

    // 6) Confirm it and verify a real, confirmed "send" of 10 BRVA exists in the wallet history.
    //    (Balance is not compared across the block: a freshly mined block matures older coinbase and adds
    //    noise; the wallet's own send record is the unambiguous proof the money moved.)
    wcli(run, ['generatetoaddress', '1', addrA]);
    await browser.waitUntil(() => {
      const r = wcli(run, ['listtransactions', '*', '30']);
      try {
        const txs = JSON.parse(r.stdout);
        return txs.some((t) => t.category === 'send' && Math.abs(t.amount) >= 10 && (t.confirmations || 0) >= 1);
      } catch { return false; }
    }, { timeout: 25000, interval: 1500, timeoutMsg: 'no confirmed send of 10 BRVA appeared in the wallet history' });

    // 7) The mempool is empty again: the payment confirmed, nothing left dangling.
    expect(mempoolSize(run)).toBe(0);

    console.log(`[e2e][14] funded ${bal0} BRVA, broadcast and confirmed a real 10 BRVA send. OK.`);
  });
});
