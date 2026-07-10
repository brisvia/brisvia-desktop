// Backend simulado (mock) para los tests E2E del frontend.
//
// What it does: injects window.__TAURI__.core.invoke BEFORE the page scripts run.
// This way tauri-bridge.js detects that "Tauri is present" and builds the REAL window.brisvia (isReal: true),
// exactamente el mismo puente que usa la app instalada. app.js corre sin cambios contra respuestas
// realistic simulated ones. This tests the UI and the frontend logic (including the password screen
// y el modo espera) sin depender del backend Rust ni de bitcoind.
//
// Honest limitation: mocking invoke does NOT exercise the real wpkh descriptor generator of the
// backend. Esa parte (el bug de la llave) se cubre aparte con el test de Rust
// `wallet_key_tests::ext_key_prefix_matches_build_network` (npm run test:rust).
'use strict';

// Real BIP39 test phrase (12 words, standard vector). Serves as a simulated "created wallet".
const DEMO_WORDS = [
  'legal', 'winner', 'thank', 'year', 'wave', 'sausage',
  'worth', 'useful', 'legal', 'winner', 'thank', 'yellow',
];

// Installs the mock on the page with a scenario configuration.
// config:
//   network       'brisvia' (red real/mainnet) | 'brisvia-test' (red de prueba)
//   walletReady   true  -> arranca en la Billetera (billetera existente)
//                 false -> arranca en el alta (onboarding) para crear billetera
//   walletOnDisk  refleja si hay billetera en disco (decide onboarding)
//   createWords   words that "create wallet" returns (success)
//   createError   if set, "create wallet" FAILS with this text (simulates a wpkh regression)
async function installMock(page, config) {
  const cfg = Object.assign(
    {
      network: 'brisvia',
      walletReady: false,
      walletOnDisk: false,
      createWords: DEMO_WORDS,
      createError: null,
    },
    config || {},
  );

  await page.addInitScript((c) => {
    // Respuestas por comando. Cada una imita la forma real que devuelve el backend Rust.
    const responders = {
      // --- Arranque / sistema ---
      system_locale: () => 'es',
      set_language: () => ({ ok: true }),
      app_version: () => '1.0.0',
      check_update: () => ({ available: false, currentVersion: '1.0.0' }),
      open_url: () => ({ ok: true }),

      // --- Estado del nodo ---
      node_status: () => ({
        connected: true,
        walletReady: c.walletReady,
        walletOnDisk: c.walletOnDisk,
      }),
      node_info: () => ({
        connected: true,
        network: c.network, // 'brisvia' => build mainnet (dispara modo espera antes del 1-ago-2026)
        chain: c.network === 'brisvia' ? 'main' : 'test',
        blocks: 0,
        headers: 0,
        peers: 3,
        difficulty: 1,
        ibd: false, // not syncing
        verificationprogress: 1,
        bestblockhash: '0'.repeat(64),
        networkhashps: 0,
      }),

      // --- Minero ---
      miner_status: () => ({
        mining: false,
        hashrate: 0,
        accepted: 0,
        secondsMining: 0,
        intensity: '50',
        threads: 0,
        cores: 8,
        totalSeconds: 0,
      }),
      miner_start: () => ({ mining: true }),
      miner_stop: () => ({ mining: false }),
      miner_set_intensity: (a) => ({ intensity: (a && a.intensity) || '50' }),

      // --- Billetera ---
      wallet_exists: () => c.walletReady,
      wallet_create_bip39: () => {
        // Simula el fallo del backend (p. ej. el bug "wpkh(): key '...' is not valid").
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
      wallet_kind: () => ({ kind: 'bip39', encrypted: true, has_seed_phrase: true }),
      tx_detail: () => ({ txid: 'demo', amount: 0, confirmations: 0, blockheight: 0, time: 0 }),
      wallet_backup: () => ({ ok: true, path: 'C:/demo/brisvia-wallet.dat' }),

      // --- Logros / ajustes ---
      achievements: () => ({ list: [], justUnlocked: [] }),
      settings_get: () => ({ autostart: false, tray: true, defaultIntensity: '50', miningMode: 'solo' }),
      settings_set: (a) => ({ ok: true, key: a && a.key, value: a && a.value }),
    };

    const invoke = (cmd, args) => {
      const fn = responders[cmd];
      if (!fn) return Promise.resolve(null); // comando desconocido: como el backend real, no rompe
      try {
        return Promise.resolve(fn(args));
      } catch (e) {
        return Promise.reject(e);
      }
    };

    // withGlobalTauri: true -> la app real espera window.__TAURI__.core.invoke. Lo replicamos.
    window.__TAURI__ = { core: { invoke } };
  }, cfg);
}

// Hooks the capture of console errors and page exceptions.
// Devuelve un array que se va llenando; los tests lo revisan al final para exigir "sin errores".
// Se ignoran los 404 de favicon/recursos opcionales que no afectan el funcionamiento.
function captureErrors(page) {
  const errors = [];
  page.on('console', (msg) => {
    if (msg.type() === 'error') {
      const text = msg.text();
      if (/favicon/i.test(text)) return; // ruido irrelevante
      errors.push('console.error: ' + text);
    }
  });
  page.on('pageerror', (err) => {
    errors.push('pageerror: ' + (err && err.message ? err.message : String(err)));
  });
  return errors;
}

module.exports = { installMock, captureErrors, DEMO_WORDS };
