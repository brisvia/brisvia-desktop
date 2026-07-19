// Simulated (mock) backend for the frontend E2E tests.
//
// What it does: injects window.__TAURI__.core.invoke BEFORE the page scripts run. That way
// tauri-bridge.js detects that "Tauri is there" and builds the REAL window.brisvia (isReal: true),
// exactly the same bridge the installed app uses. app.js runs unchanged against realistic simulated
// responses. This tests the UI and the frontend logic (including the password screen and wait mode)
// without depending on the Rust backend or bitcoind.
//
// Honest limitation: mocking invoke does NOT exercise the backend's real wpkh descriptor generator.
// That part (the key bug) is covered separately by the Rust test
// `wallet_key_tests::ext_key_prefix_matches_build_network` (npm run test:rust).
'use strict';

// Real BIP39 test phrase (12 words, standard vector). Serves as a simulated "created wallet".
const DEMO_WORDS = [
  'legal', 'winner', 'thank', 'year', 'wave', 'sausage',
  'worth', 'useful', 'legal', 'winner', 'thank', 'yellow',
];

// Installs the mock on the page with a scenario configuration.
// config:
//   network       'brisvia' (real network/mainnet) | 'brisvia-test' (test network)
//   walletReady   true  -> starts on the Wallet (existing wallet)
//                 false -> starts on onboarding to create a wallet
//   walletOnDisk  reflects whether a wallet is on disk (decides onboarding)
//   createWords   words returned by "create wallet" (success)
//   createError   if set, "create wallet" FAILS with this text (simulates a wpkh regression)
async function installMock(page, config) {
  const cfg = Object.assign(
    {
      network: 'brisvia',
      walletReady: false,
      walletOnDisk: false,
      createWords: DEMO_WORDS,
      createError: null,
      // Auto-start-at-launch scenario knobs:
      //   mainnetInMs   launch instant relative to now (ms). Positive = future (wait mode); negative = already past.
      //   mainnetAbsMs  absolute launch instant (ms); overrides mainnetInMs when set.
      //   ibd           node still syncing (SOLO dependency not ready) when true.
      //   poolConnected / poolSuspended  pool availability (POOL dependency).
      //   autoStart / autoIntensity  the persisted auto-start choice the backend would return on startup.
      mainnetInMs: 3600000, // default: 1 h in the future -> wait mode
      mainnetAbsMs: null,
      ibd: false,
      poolConnected: false,
      poolSuspended: false,
      autoStart: false,
      autoIntensity: 'equilibrado',
    },
    config || {},
  );

  await page.addInitScript((c) => {
    // Minimal in-page state so mode switches and start/stop are OBSERVABLE across calls (the real backend is
    // stateful; a stateless mock can't test a SOLO<->POOL switch). The reported mode is normalised exactly like
    // the backend: never "pool" while the pool is not enabled.
    const S = { mining: false, mode: (c.miningMode || 'solo'), poolEnabled: c.poolEnabled === true,
                autoStart: c.autoStart === true, autoIntensity: c.autoIntensity || 'equilibrado' };
    const reportedMode = () => (S.mode === 'pool' && S.poolEnabled ? 'pool' : 'solo');
    // The launch instant is a FIXED point captured once when the page loads (like the real backend constant), so
    // real time advancing past it crosses the boundary — that is how the "hot unlock" is tested without a restart.
    const launchAt = (typeof c.mainnetAbsMs === 'number') ? c.mainnetAbsMs : Date.now() + c.mainnetInMs;
    // Responses per command. Each mimics the real shape the Rust backend returns.
    const responders = {
      // --- Startup / system ---
      system_locale: () => 'es',
      set_language: () => ({ ok: true }),
      app_version: () => '1.0.0',
      check_update: () => ({ available: false, currentVersion: '1.0.0' }),
      open_url: () => ({ ok: true }),

      // --- Node state ---
      node_status: () => ({
        connected: c.connected !== false, // default true; set connected:false to simulate a down/unreachable node
        walletReady: c.walletReady,
        walletOnDisk: c.walletOnDisk,
      }),
      node_info: () => ({
        connected: true,
        network: c.network, // 'brisvia' => mainnet build (triggers wait mode before 2026-08-01)
        chain: c.network === 'brisvia' ? 'main' : 'test',
        blocks: 0,
        headers: 0,
        peers: 3,
        difficulty: 1,
        ibd: c.ibd === true, // still syncing when the scenario sets ibd (SOLO dependency not ready)
        verificationprogress: c.ibd === true ? 0.5 : 1,
        bestblockhash: '0'.repeat(64),
        networkhashps: 0,
      }),

      // --- Miner ---
      miner_status: () => ({
        mining: S.mining,
        mode: reportedMode(), // REAL active mode, normalised (never "pool" while the pool is disabled)
        pool: {
          enabled: S.poolEnabled,
          connected: c.poolConnected === true,
          suspended: c.poolSuspended === true,
          phase: S.mining && reportedMode() === 'pool' ? 'working'
            : (c.poolSuspended === true ? 'suspended' : 'disconnected'),
          retrySecs: c.poolSuspended === true ? 45 : 0,
          hasJob: false,
          sharesSent: 0,
          sharesAccepted: 0,
          sharesRejected: 0,
        },
        hashrate: 0,
        accepted: 0,
        secondsMining: 0,
        intensity: '50',
        threads: 0,
        cores: 8,
        // Auto-start-at-launch fields the frontend reads: the canonical launch instant + the persisted choice.
        mainnetStartMs: launchAt,
        autoStart: S.autoStart,
        autoIntensity: S.autoIntensity,
        totalSeconds: 0,
      }),
      miner_start: () => { S.mining = true; return { mining: true }; },
      miner_stop: () => { S.mining = false; return { mining: false }; },
      miner_set_intensity: (a) => ({ intensity: (a && a.intensity) || '50' }),

      // --- Wallet ---
      // LOCAL existence check (reads wallet_seed.enc, independent of the node). Defaults to walletReady so the
      // existing scenarios keep behaving the same; set seedOnDisk explicitly to test the wallet-vs-node cases.
      wallet_seed_on_disk: () => (c.seedOnDisk !== undefined ? c.seedOnDisk : c.walletReady),
      // Legacy classification (B). Default encrypted_present; set legacyStatus:'legacy_corrupt' to test recovery.
      wallet_legacy_status: () => ({ status: c.legacyStatus || 'encrypted_present' }),
      wallet_exists: () => c.walletReady,
      wallet_create_bip39: () => {
        // Simulates the backend failure (e.g. the "wpkh(): key '...' is not valid" bug).
        if (c.createError) return Promise.reject(c.createError);
        return { words: c.createWords, fingerprint: '1a2b3c4d' };
      },
      wallet_restore_bip39: () => ({ ok: true }),
      wallet_verify_backup: () => ({ ok: true }),
      wallet_check_backup: () => ({ ok: true }),
      wallet_confirm_backup: () => ({ backed_up: true }),
      wallet_seed: () => c.createWords,
      wallet_reveal_seed: () => ({ words: c.createWords }),
      wallet_migrate_encrypt: () => ({ ok: true }),
      wallet_summary: () => ({
        balance: 0,
        immature: 0,
        incoming: 0,
        pending: 0,
        address: 'brv1qdemoaddress000000000000000000000000000',
        backed_up: true,
      }),
      wallet_history: () => [],
      wallet_new_address: () => ({ address: 'brv1qdemoaddress000000000000000000000000000' }),
      wallet_addresses: () => [{ address: 'brv1qdemoaddress000000000000000000000000000', balance: 0 }],
      wallet_send: () => ({ ok: false, error: 'ERR:INSUFFICIENT_FUNDS' }),
      wallet_kind: () => { if (c.kindFails) throw 'ERR:NODE_NOT_READY'; return { kind: 'bip39', encrypted: c.encrypted !== false, has_seed_phrase: true }; },
      tx_detail: () => ({ txid: 'demo', amount: 0, confirmations: 0, blockheight: 0, time: 0 }),
      wallet_backup: () => ({ ok: true, path: 'C:/demo/brisvia-wallet.dat' }),

      // --- Achievements / settings ---
      achievements: () => ({ list: [], justUnlocked: [] }),
      settings_get: () => ({ autostart: false, tray: true, defaultIntensity: '50', miningMode: S.mode }),
      settings_set: (a) => { if (a && a.key === 'miningMode') S.mode = a.value; return { ok: true, key: a && a.key, value: a && a.value }; },
      // Arm/disarm the voluntary auto-start (persisted in the stateful mock so a reload could re-read it).
      mining_set_autostart: (a) => { S.autoStart = !!(a && a.enabled); if (a && a.intensity) S.autoIntensity = a.intensity; return { ok: true, autoStart: S.autoStart, autoIntensity: S.autoIntensity }; },
    };

    const invoke = (cmd, args) => {
      const fn = responders[cmd];
      if (!fn) return Promise.resolve(null); // unknown command: like the real backend, does not break
      try {
        return Promise.resolve(fn(args));
      } catch (e) {
        return Promise.reject(e);
      }
    };

    // withGlobalTauri: true -> the real app expects window.__TAURI__.core.invoke. We replicate it.
    window.__TAURI__ = { core: { invoke } };
  }, cfg);
}

// Hooks the capture of console errors and page exceptions.
// Returns an array that fills up; the tests check it at the end to require "no errors".
// The favicon/optional-resource 404s that do not affect operation are ignored.
function captureErrors(page) {
  const errors = [];
  page.on('console', (msg) => {
    if (msg.type() === 'error') {
      const text = msg.text();
      if (/favicon/i.test(text)) return; // irrelevant noise
      errors.push('console.error: ' + text);
    }
  });
  page.on('pageerror', (err) => {
    errors.push('pageerror: ' + (err && err.message ? err.message : String(err)));
  });
  return errors;
}

module.exports = { installMock, captureErrors, DEMO_WORDS };
