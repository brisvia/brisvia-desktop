// Migration test, step 1 (runs on the INSTALLED 1.0.5, driven via tauri-driver / BRISVIA_E2E_APP).
// Creates a real wallet through the installed app's own onboarding: BIP39 phrase, encrypted seed file,
// the Core "brisvia" wallet, and the first-run state persisted under %APPDATA%\com.brisvia.miner.
// It does NOT mine or fund: the sealed installers are mainnet builds (real PoW, pre-launch), so the
// funding + confirmed-transaction survival is covered by the regtest E2E transaction test (#14) on the
// e2e binary, which shares the exact same wallet module and byte-identical seed format.
// The password is a throwaway test constant; the phrase is never written to disk by this test.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Migration prepare — create a wallet on the installed 1.0.5', () => {
  it('walks onboarding on the installed app and lands on the wallet', async () => {
    const seed = await harness.onboardCreate(PASSWORD); // works in wait mode (only mining is on hold pre-launch)
    expect(seed.length).toBe(12);
    // The wallet view being up means the encrypted seed + Core wallet were written to the real datadir.
    await (await $('[data-testid="view-wallet"]')).waitForDisplayed({ timeout: 15000 });
  });
});
