// Tauri bridge: exposes window.brisvia (wallet + mining + settings) over the Tauri commands.
// The UI (app.js) is the same in Tauri and in a preview browser: with no Tauri, it uses a deterministic
// in-memory mock (same methods). The real backend (Brisvia bitcoind over RPC, mining engine, autostart, tray)
// is wired by the invoke block below, without touching app.js.
(function () {
  const invoke = window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;

  if (invoke) {
    // ----- Real backend (Tauri). Commands that don't exist yet fall to the catch -> the UI shows empty. -----
    const call = (cmd, args) => invoke(cmd, args).catch(() => null);
    window.brisvia = {
      isReal: true, // running inside Tauri with the real node
      nodeStatus: () => call('node_status'),
      nodeInfo: () => call('node_info'),
      openUrl: (url) => call('open_url', { url }),
      systemLocale: () => call('system_locale'),
      setLanguage: (lang) => call('set_language', { lang }),
      checkUpdate: () => call('check_update'),
      installUpdate: () => invoke('install_update'),
      getStatus: () => invoke('miner_status'),
      start: (intensity) => invoke('miner_start', { intensity }),
      stop: () => invoke('miner_stop'),
      setIntensity: (intensity) => invoke('miner_set_intensity', { intensity }),
      wallet: {
        exists: () => call('wallet_exists'),
        create: () => call('wallet_create_bip39', { name: 'brisvia' }),
        verifyBackup: (words) => call('wallet_verify_backup', { words }),
        restore: (phrase) => call('wallet_restore_bip39', { phrase, name: 'brisvia' }),
        getSeed: () => call('wallet_seed'),
        confirmBackup: () => call('wallet_confirm_backup'),
        summary: () => call('wallet_summary'),
        history: () => call('wallet_history'),
        newAddress: () => call('wallet_new_address'),
        send: (address, amount) => call('wallet_send', { address, amount }),
        txDetail: (txid) => call('tx_detail', { txid }),
        backup: () => call('wallet_backup'),
        kind: () => call('wallet_kind'),
      },
      settings: {
        get: () => call('settings_get'),
        set: (key, value) => call('settings_set', { key, value }),
      },
    };
    return;
  }

  // ----- Preview mock (browser without Tauri) -----
  const WORDS = ('abandon ability able about above absent absorb abstract absurd abuse access accident account ' +
    'accuse achieve acid acoustic acquire across action actor actual adapt add addict address adjust admit adult ' +
    'advance advice aerobic affair afford afraid again agent agree ahead aim air airport aisle alarm album alcohol ' +
    'alert alien alley allow almost alone alpha already also alter always amateur amazing among amount amused ' +
    'anchor ancient anger angle angry animal ankle announce annual another answer antenna antique anxiety apart ' +
    'apology appear apple approve april arch arctic area arena argue arm armor army around arrange arrest arrive ' +
    'arrow art artist artwork ask aspect asset assist assume athlete atom attack attend attitude attract auction ' +
    'audit august aunt author auto autumn average avocado avoid awake aware away awesome awful awkward axis').split(' ');

  const LS = {
    get: (k, d) => { try { const v = localStorage.getItem(k); return v === null ? d : JSON.parse(v); } catch { return d; } },
    set: (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch {} },
  };

  function genSeed() {
    const out = [], used = new Set(), rnd = new Uint32Array(48);
    if (window.crypto && window.crypto.getRandomValues) window.crypto.getRandomValues(rnd);
    else for (let i = 0; i < 48; i++) rnd[i] = (i * 2654435761 + 12345) >>> 0;
    for (let i = 0; i < rnd.length && out.length < 12; i++) {
      const w = WORDS[rnd[i] % WORDS.length];
      if (!used.has(w)) { used.add(w); out.push(w); } // no repeated words
    }
    for (let i = 0; out.length < 12; i++) if (!used.has(WORDS[i])) { used.add(WORDS[i]); out.push(WORDS[i]); }
    return out;
  }
  function genAddress() {
    const hex = '0123456789acdefghjklmnpqrstuvwxyz';
    let s = 'brv1q';
    const rnd = new Uint8Array(38);
    if (window.crypto && window.crypto.getRandomValues) window.crypto.getRandomValues(rnd);
    for (let i = 0; i < 38; i++) s += hex[rnd[i] % hex.length];
    return s;
  }

  let st = { mining: false, seconds: 0, intensity: 'equilibrado' };
  setInterval(() => { if (st.mining) st.seconds++; }, 1000);

  window.brisvia = {
    getStatus: async () => {
      const base = { suave: 120, equilibrado: 320, intenso: 620 }[st.intensity] || 320;
      const cores = 8, threads = { suave: 1, equilibrado: 4, intenso: 8 }[st.intensity] || 4;
      return { mining: st.mining, hashrate: st.mining ? base : 0, accepted: Math.floor(st.seconds / 12), secondsMining: st.seconds, intensity: st.intensity, threads: st.mining ? threads : 0, cores, totalSeconds: st.seconds };
    },
    start: async (i) => { st.mining = true; if (i) st.intensity = i; return { mining: true }; },
    stop: async () => { st.mining = false; return { mining: false }; },
    setIntensity: async (i) => { st.intensity = i; return { intensity: i }; },

    wallet: {
      exists: async () => LS.get('brv_wallet', null) !== null,
      create: async () => {
        const wallet = { seed: genSeed(), address: genAddress(), backed_up: false, created: Date.now() };
        LS.set('brv_wallet', wallet);
        return { words: wallet.seed, fingerprint: 'demo0000' };
      },
      verifyBackup: async (words) => { const seed = LS.get('brv_wallet', {}).seed || []; return { ok: JSON.stringify(seed) === JSON.stringify(words) }; },
      restore: async () => { const wallet = { seed: genSeed(), address: genAddress(), backed_up: true, created: Date.now() }; LS.set('brv_wallet', wallet); return { ok: true }; },
      getSeed: async () => (LS.get('brv_wallet', {}).seed || []),
      confirmBackup: async () => { const w = LS.get('brv_wallet', {}); w.backed_up = true; LS.set('brv_wallet', w); return { backed_up: true }; },
      summary: async () => {
        const w = LS.get('brv_wallet', {});
        return { balance: 0, immature: 0, incoming: 0, pending: 0, address: w.address || '', backed_up: !!w.backed_up };
      },
      history: async () => [],
      newAddress: async () => { const w = LS.get('brv_wallet', {}); w.address = genAddress(); LS.set('brv_wallet', w); return { address: w.address }; },
      send: async () => ({ ok: false, error: 'ERR:INSUFFICIENT_FUNDS' }),
      txDetail: async () => ({ txid: 'demo', amount: 0, confirmations: 0, blockheight: 0, time: 0 }),
      backup: async () => ({ ok: true, path: 'C:/Users/…/Documents/Brisvia-backups/brisvia-wallet.dat' }),
      kind: async () => ({ kind: 'preview_wallet', has_seed_phrase: false }),
    },
    nodeInfo: async () => ({ connected: true, chain: 'regtest', blocks: 12, headers: 12, peers: 0, difficulty: 0, bestblockhash: '0'.repeat(64), networkhashps: 0 }),

    settings: {
      get: async () => LS.get('brv_settings', { autostart: false, tray: true, defaultIntensity: 'equilibrado' }),
      set: async (key, value) => { const s = LS.get('brv_settings', { autostart: false, tray: true, defaultIntensity: 'equilibrado' }); s[key] = value; LS.set('brv_settings', s); return s; },
    },
    openUrl: async (url) => { window.open(url, '_blank'); },
    systemLocale: async () => navigator.language || 'en',
    setLanguage: async () => ({ ok: true }),
    checkUpdate: async () => ({ available: false, currentVersion: '0.1.0' }),
    installUpdate: async () => ({ ok: true }),
  };
})();
