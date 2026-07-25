// P0 journey #17 — Legacy wallet-layout migration, proven end to end with a REAL node.
//
// Reproduces the ACTUAL bug that produced 1.1.1: a wallet left in the OLD location (<chain>/brisvia, with no
// wallets/ directory) by an older build. The unit tests already prove the files move correctly; what they can
// NOT prove is that a real bitcoind then LOADS the migrated wallet and still owns its keys. This does exactly
// that, on the real compiled app:
//   - create a wallet (it lands in the fixed wallets/brisvia) and capture a receive address that IS ismine;
//   - stop the node and RELOCATE the wallet to the legacy layout (<chain>/brisvia, wallets/ removed);
//   - reopen the app -> prepare_wallet_layout migrates it back BEFORE the node opens it -> the node loads it;
//   - prove it was real and lossless: wallets/brisvia is back, a dated backup sits OUTSIDE wallets/, and the
//     SAME address is STILL ismine on the reloaded wallet (keys/descriptors survived byte for byte).
//
// The seed/password are throwaway and generated only for the test.
'use strict';

const fs = require('fs');
const path = require('path');
const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Journey 17 — legacy wallet layout migration (real node)', () => {
  it('migrates a wallet left in the old location and the reloaded node still owns its address', async () => {
    const run = harness.fromEnv();
    const chainDir = path.join(run.datadir, run.subdir); // <datadir>/regtest
    const canonical = path.join(chainDir, 'wallets', 'brisvia');
    const legacy = path.join(chainDir, 'brisvia');

    // 1) Create a wallet the normal way; it lands in the fixed wallets/brisvia. Capture an address the node owns.
    await harness.onboardCreate(PASSWORD);
    const addr = await harness.readReceiveAddress();
    expect(addr.length).toBeGreaterThan(10);
    const before = harness.rpc(run.datadir, run.port, ['-rpcwallet=brisvia', 'getaddressinfo', addr]);
    expect(before.status).toBe(0);
    expect(before.stdout).toContain('"ismine": true');
    expect(fs.existsSync(path.join(canonical, 'wallet.dat'))).toBe(true);

    // 2) Stop the node and wait for it to FULLY exit (Windows releases the wallet file locks on exit).
    harness.rpc(run.datadir, run.port, ['stop']);
    await harness.waitFor(() => harness.countProcs(run.datadir) === 0, {
      timeout: 30000, interval: 500, msg: 'the node to fully stop before relocating the wallet',
    });

    // 3) RELOCATE to the exact legacy layout that caused the bug: wallet in <chain>/brisvia, no wallets/ dir.
    //    Retry the move: Windows can hold the lock for an instant after the process is already gone.
    await harness.waitFor(() => {
      try {
        fs.renameSync(canonical, legacy);
        fs.rmSync(path.join(chainDir, 'wallets'), { recursive: true, force: true });
        return true;
      } catch { return false; }
    }, { timeout: 15000, interval: 500, msg: 'the wallet files to become movable after the node stops' });
    expect(fs.existsSync(path.join(legacy, 'wallet.dat'))).toBe(true);
    expect(fs.existsSync(path.join(chainDir, 'wallets'))).toBe(false);

    // 4) Reopen the app on the SAME data folder. Its startup runs prepare_wallet_layout (migrates the legacy
    //    wallet back into wallets/brisvia, with a verified copy + dated backup) BEFORE the node opens it, then
    //    loads the wallet exactly as a normal launch does (journey 13 proves reopen loads the wallet).
    await browser.reloadSession();
    const walletView = await $('[data-testid="view-wallet"]');
    const welcome = await $('[data-testid="onb-welcome"]');
    await browser.waitUntil(async () => (await walletView.isDisplayed()) || (await welcome.isDisplayed()), {
      timeout: 90000, timeoutMsg: 'the app came back to neither the wallet nor onboarding after the migration',
    });
    // Onboarding must NOT reappear: a reappearance would mean the migration lost the wallet.
    expect(await welcome.isDisplayed()).toBe(false);

    // 5) The migration was real and lossless.
    //    - the wallet is back in the fixed location;
    expect(fs.existsSync(path.join(canonical, 'wallet.dat'))).toBe(true);
    //    - a dated backup was kept OUTSIDE wallets/ (so listwalletdir never enumerates it as a second wallet);
    const backups = fs.readdirSync(chainDir).filter((n) => n.startsWith('brisvia.legacy-backup-'));
    expect(backups.length).toBeGreaterThan(0);
    //    - and the SAME address is STILL ismine on the reloaded node: keys/descriptors survived byte for byte.
    await harness.waitRpcUp(run.datadir, run.port, 120000);
    await harness.waitFor(() => {
      const r = harness.rpc(run.datadir, run.port, ['-rpcwallet=brisvia', 'getaddressinfo', addr]);
      return r.status === 0 && r.stdout.includes('"ismine": true');
    }, { timeout: 40000, interval: 1000, msg: 'the reloaded wallet to still own the original address' });
  });
});
