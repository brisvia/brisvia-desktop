// Brisvia app UI (wallet + mining + settings). Talks to the backend only through window.brisvia.
// Every visible text comes from the i18n system (window.I18N + data-i18n in the HTML).
const $ = (s) => document.querySelector(s);
const $$ = (s) => Array.from(document.querySelectorAll(s));
const T = (k, v) => window.I18N.t(k, v);
// Maps backend error codes ("ERR:CODE") to the active language; unknown ones pass through unchanged.
function transError(err) {
  if (typeof err === 'string' && err.startsWith('ERR:')) {
    const map = {
      INSUFFICIENT_FUNDS: 'errors.insufficient_funds', INVALID_ADDRESS: 'errors.invalid_address',
      FEE_TOO_LOW: 'errors.fee_too_low', NODE_STARTING: 'errors.node_starting', NODE_UNAVAILABLE: 'errors.node_unavailable',
      BAD_PASSWORD: 'errors.bad_password', WEAK_PASSWORD: 'errors.weak_password',
      NO_SEED_FILE: 'errors.no_seed_file', SEED_CORRUPT: 'errors.seed_corrupt',
      INVALID_PHRASE: 'errors.invalid_phrase', ENCRYPT_FAILED: 'errors.encrypt_failed',
      WALLET_EXISTS: 'errors.wallet_exists',
      NODE_SYNCING: 'errors.node_syncing', NO_PEERS: 'errors.no_peers', CLOCK_SKEW: 'errors.clock_skew',
      WALLET_NOT_READY: 'errors.wallet_not_ready', MINER_NOT_FOUND: 'errors.miner_not_found',
      POOL_ADDR_MISSING: 'errors.pool_addr_missing',
    };
    const key = map[err.slice(4)];
    if (key) return T(key);
  }
  return err || T('errors.generic');
}

// ===================== View navigation =====================
function showView(name) {
  $$('.view').forEach((v) => (v.hidden = v.dataset.view !== name));
  $$('.nav-btn').forEach((b) => b.classList.toggle('active', b.dataset.view === name));
  if (name === 'wallet') loadWallet();
  if (name === 'settings') loadSettings();
  if (name === 'achievements') loadAchievements();
}
$$('.nav-btn').forEach((b) => b.addEventListener('click', () => showView(b.dataset.view)));

// ===================== First run (onboarding + wallet + seed) =====================
const ONB = ['welcome1', 'welcome2', 'welcome3'];
let onbStep = 0;
function setupStep(name) { $$('#setup .step').forEach((s) => (s.hidden = s.dataset.step !== name)); }
function renderOnb() {
  $('#onb-title').textContent = T('onboarding.' + ONB[onbStep] + '_t');
  $('#onb-text').textContent = T('onboarding.' + ONB[onbStep] + '_x');
  $$('.onb-dots .dot').forEach((d, i) => d.classList.toggle('on', i === onbStep));
  $('#onb-next').textContent = onbStep === ONB.length - 1 ? T('onboarding.start') : T('onboarding.next');
}
$('#onb-next').addEventListener('click', () => {
  if (onbStep < ONB.length - 1) { onbStep++; renderOnb(); }
  else setupStep('choose');
});

// -- Create wallet (the backend generates the real 12 words) --
let currentSeed = [];
$('#btn-create').addEventListener('click', () => startPassStep('create'));
$('#btn-import').addEventListener('click', () => { buildImportGrid(); setupStep('import'); });
function alertInline(msg) { const t = $('#onb-text'); if (t) t.textContent = msg; }

// -- Password step: encrypts the wallet (Core) and the 12-word phrase. Used for both create and import. --
let passMode = 'create';
let importedWords = [];
function startPassStep(mode) {
  passMode = mode;
  $('#pass-1').value = ''; $('#pass-2').value = ''; $('#pass-msg').hidden = true;
  $('#pass-show').checked = false; $('#pass-1').type = 'password'; $('#pass-2').type = 'password';
  updatePassMeter();
  setupStep('pass');
  $('#pass-1').focus();
}
// Strength 0..4: length + variety. A visual guide, not a hard block (the real backup is the 12 words).
function passStrength(p) {
  let s = 0;
  if (p.length >= 6) s++;   // minimo permitido
  if (p.length >= 12) s++;
  if (/[A-Z]/.test(p) && /[a-z]/.test(p)) s++;
  if (/\d/.test(p)) s++;
  if (/[^A-Za-z0-9]/.test(p)) s++;
  return Math.min(s, 4);
}
function updatePassMeter() {
  const p = $('#pass-1').value;
  $('#pass-meter').className = 'pass-meter lvl-' + (p ? passStrength(p) : 0);
}
$('#pass-1').addEventListener('input', updatePassMeter);
// Clear a stale error as soon as the user edits either field, so a previous
// "at least 8 characters" / "passwords don't match" message does not linger
// after it has already been corrected.
const clearPassMsg = () => { const m = $('#pass-msg'); if (m && !m.hidden) m.hidden = true; };
$('#pass-1').addEventListener('input', clearPassMsg);
$('#pass-2').addEventListener('input', clearPassMsg);
$('#pass-show').addEventListener('change', (e) => {
  const t = e.target.checked ? 'text' : 'password';
  $('#pass-1').type = t; $('#pass-2').type = t;
});
$('#pass-back').addEventListener('click', () => setupStep(passMode === 'import' ? 'import' : 'choose'));
$('#pass-next').addEventListener('click', async () => {
  const p1 = $('#pass-1').value, p2 = $('#pass-2').value;
  const msg = $('#pass-msg'); msg.hidden = false; msg.className = 'verify-msg err';
  if ([...p1].length < 6) { msg.textContent = T('onboarding.pass_weak'); return; } // [...] counts code points, matching backend MIN_PASSWORD_LEN
  if (p1 !== p2) { msg.textContent = T('onboarding.pass_mismatch'); return; }
  const btn = $('#pass-next'); btn.disabled = true; btn.textContent = T('onboarding.creating');
  try {
    if (passMode === 'create') {
      const r = await window.brisvia.wallet.create(p1);
      if (r && r.words && r.words.length === 12) { currentSeed = r.words; msg.hidden = true; showSeedStep(); }
      else { msg.textContent = r && r.error ? transError(r.error) : T('onboarding.create_err'); }
    } else {
      const r = await window.brisvia.wallet.restore(importedWords.join(' '), p1);
      if (r && r.ok) { localStorage.setItem('brisvia_onboarded', '1'); msg.hidden = true; finishSetup(); }
      else { msg.textContent = r && r.error ? transError(r.error) : T('onboarding.import_err'); }
    }
  } finally {
    btn.disabled = false; btn.textContent = T('common.continue');
  }
});

// 12 boxes to import the phrase (one per word). Pasting all 12 at once into the first box spreads them out.
function buildImportGrid() {
  const grid = $('#import-grid');
  grid.innerHTML = '';
  for (let i = 0; i < 12; i++) {
    const li = document.createElement('li');
    const inp = document.createElement('input');
    inp.type = 'text'; inp.autocomplete = 'off'; inp.spellcheck = false; inp.setAttribute('aria-label', `${i + 1}`);
    inp.addEventListener('paste', (e) => {
      const txt = (e.clipboardData || window.clipboardData).getData('text');
      const words = txt.trim().toLowerCase().split(/\s+/).filter(Boolean);
      if (words.length > 1) {
        e.preventDefault();
        const inputs = grid.querySelectorAll('input');
        words.slice(0, 12).forEach((w, k) => { if (inputs[k]) inputs[k].value = w; });
        const next = grid.querySelectorAll('input')[Math.min(words.length, 11)];
        if (next) next.focus();
      }
    });
    inp.addEventListener('keydown', (e) => {
      if ((e.key === ' ' || e.key === 'Enter') && inp.value.trim()) {
        e.preventDefault();
        const inputs = [...grid.querySelectorAll('input')];
        const idx = inputs.indexOf(inp);
        if (inputs[idx + 1]) inputs[idx + 1].focus();
      }
    });
    li.appendChild(inp);
    grid.appendChild(li);
  }
}

// -- Show the seed (the 12 words already generated by the backend) --
function showSeedStep() {
  const grid = $('#seed-grid');
  grid.innerHTML = '';
  currentSeed.forEach((w) => { const li = document.createElement('li'); li.textContent = w; grid.appendChild(li); });
  $('#seed-ack').checked = false;
  $('#seed-next').disabled = true;
  setupStep('seed');
}
$('#seed-ack').addEventListener('change', (e) => { $('#seed-next').disabled = !e.target.checked; });
$('#seed-next').addEventListener('click', () => buildVerify());

// -- Backup verification --
let verifyPositions = [], verifyExpected = [], verifyFilled = [];
function shuffle(a) { const r = a.slice(); for (let i = r.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [r[i], r[j]] = [r[j], r[i]]; } return r; }
function buildVerify() {
  verifyPositions = shuffle(currentSeed.map((_, i) => i)).slice(0, 3).sort((a, b) => a - b);
  verifyExpected = verifyPositions.map((i) => currentSeed[i]);
  verifyFilled = [];
  const distractors = shuffle(currentSeed.filter((_, i) => !verifyPositions.includes(i))).slice(0, 3);
  const bank = shuffle([...verifyExpected, ...distractors]);
  renderVerify(bank);
  $('#verify-msg').hidden = true;
  setupStep('verify');
}
function renderVerify(bank) {
  const slots = $('#verify-slots'); slots.innerHTML = '';
  verifyPositions.forEach((pos, idx) => {
    const d = document.createElement('div');
    d.className = 'slot' + (verifyFilled[idx] ? ' filled' : '');
    d.innerHTML = `<span class="slot-n">${pos + 1}</span> <span>${verifyFilled[idx] || '—'}</span>`;
    slots.appendChild(d);
  });
  if (bank) {
    const bk = $('#verify-bank'); bk.innerHTML = '';
    bank.forEach((w) => {
      const c = document.createElement('button');
      c.className = 'chip'; c.textContent = w;
      c.addEventListener('click', () => pickWord(w, c));
      bk.appendChild(c);
    });
  }
}
function pickWord(word, chip) {
  if (verifyFilled.length >= verifyPositions.length) return;
  verifyFilled.push(word);
  chip.classList.add('used');
  renderVerify(null);
  if (verifyFilled.length === verifyPositions.length) checkVerify();
}
async function checkVerify() {
  const ok = verifyFilled.every((w, i) => w === verifyExpected[i]);
  const msg = $('#verify-msg');
  msg.hidden = false;
  $$('#verify-slots .slot').forEach((s, i) => s.classList.add(verifyFilled[i] === verifyExpected[i] ? 'ok' : 'err'));
  if (ok) {
    msg.textContent = T('onboarding.verify_ok');
    msg.className = 'verify-msg ok';
    if (window.brisvia.wallet.verifyBackup) await window.brisvia.wallet.verifyBackup(currentSeed);
    else await window.brisvia.wallet.confirmBackup();
    localStorage.setItem('brisvia_onboarded', '1');
    setTimeout(finishSetup, 900);
  } else {
    msg.textContent = T('onboarding.verify_err');
    msg.className = 'verify-msg err';
  }
}
$('#verify-reset').addEventListener('click', buildVerify);
$('#verify-back').addEventListener('click', showSeedStep);

// -- Import --
$('#import-back').addEventListener('click', () => setupStep('choose'));
// Clear a stale "must be 12 words" error as soon as the user edits any word.
$('#import-grid').addEventListener('input', () => { const m = $('#import-msg'); if (m && !m.hidden) m.hidden = true; });
$('#import-ok').addEventListener('click', async () => {
  // Split each cell by spaces too, so "13 words pasted into 12 cells" is caught here (audit N3).
  const words = [...$('#import-grid').querySelectorAll('input')]
    .flatMap((i) => i.value.trim().toLowerCase().split(/\s+/)).filter(Boolean);
  const msg = $('#import-msg');
  if (words.length !== 12) {
    msg.hidden = false; msg.className = 'verify-msg err';
    msg.textContent = T('onboarding.import_len_err', { n: words.length });
    return;
  }
  // Validate the phrase HERE, before asking for a password, so a bad word is flagged on the words screen
  // and not later on the password screen (audit N3). If the check can't run, fall through (backend re-checks).
  let phraseValid = true;
  try { phraseValid = await window.brisvia.wallet.validatePhrase(words); } catch { phraseValid = true; }
  if (!phraseValid) {
    msg.hidden = false; msg.className = 'verify-msg err';
    msg.textContent = T('errors.invalid_phrase');
    return;
  }
  msg.hidden = true;
  importedWords = words;
  startPassStep('import'); // ask for a password to encrypt the restored wallet
});

function finishSetup() { $('#setup').hidden = true; showView('wallet'); if (window.brisvia.isReal) loadWallet(); }

// ===================== Mining =====================
let mining = false;
// Formats a hashrate (H/s) with the right unit, and a duration (seconds) in a human, non-technical way.
function fmtHashrate(hs) {
  hs = hs || 0;
  if (hs >= 1e9) return `${(hs / 1e9).toFixed(2)} <span class="unit">GH/s</span>`;
  if (hs >= 1e6) return `${(hs / 1e6).toFixed(2)} <span class="unit">MH/s</span>`;
  if (hs >= 1000) return `${(hs / 1000).toFixed(2)} <span class="unit">kH/s</span>`;
  return `${Math.round(hs)} <span class="unit">H/s</span>`;
}
// Mining time, broken down into days / hours / minutes / seconds. Once there are days, all four levels
// are shown; below a day we start at the largest non-zero unit so we never print leading "0d 0h".
function fmtDuration(secs) {
  secs = Math.floor(secs || 0);
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const u = (val, key) => `${val} <span class="unit">${T('mine.' + key)}</span>`;
  if (d > 0) return `${u(d, 'unit_d')} ${u(h, 'unit_h')} ${u(m, 'unit_min')} ${u(s, 'unit_s')}`;
  if (h > 0) return `${u(h, 'unit_h')} ${u(m, 'unit_min')} ${u(s, 'unit_s')}`;
  if (m > 0) return `${u(m, 'unit_min')} ${u(s, 'unit_s')}`;
  return u(s, 'unit_s');
}
async function refreshMine() {
  const s = await window.brisvia.getStatus();
  mining = s.mining;
  // Suppress the "Preparing…" flash for a few seconds after a live power change (the engine relaunches behind the scenes).
  const preparing = mining && s.preparing && Date.now() > suppressPreparingUntil;
  const toggle = $('#toggle');
  // WAIT MODE (real-network build, before the Aug 1, 2026 15:00 UTC launch): the wallet works normally,
  // but the "Mine" button stays disabled until the network opens. Fully automatic by date; nothing to toggle.
  // This is a UX convenience only — the real guarantee is the network consensus.
  if (isWaitMode()) {
    $('#state-badge').textContent = T('wait.badge');
    $('#state-badge').className = 'badge prep';
    $('#hero-title').textContent = T('wait.title');
    $('#hero-sub').textContent = T('wait.sub') + ' — ' + launchCountdownText();
    toggle.textContent = T('mine.start');
    toggle.className = 'btn giant primary';
    toggle.disabled = true;
    return;
  }
  // While the node catches up with the network (and is not already mining), mining is not allowed yet.
  if (syncing && !mining) {
    $('#state-badge').textContent = T('net.syncing');
    $('#state-badge').className = 'badge prep';
    $('#hero-title').textContent = T('sync.title');
    $('#hero-sub').textContent = syncProgress > 0 ? T('sync.sub') + ' — ' + T('sync.progress', { p: Math.floor(syncProgress * 100) }) : T('sync.sub');
    toggle.textContent = T('mine.start');
    toggle.className = 'btn giant primary';
    toggle.disabled = true;
    return;
  }
  toggle.disabled = false;
  // Three states: stopped / preparing (building the dataset, a few seconds) / participating.
  if (preparing) {
    $('#state-badge').textContent = T('mine.preparing');
    $('#state-badge').className = 'badge prep';
    $('#hero-title').textContent = T('mine.preparing_title');
    $('#hero-sub').textContent = T('mine.preparing_sub');
  } else {
    $('#state-badge').textContent = mining ? T('mine.participating') : T('mine.stopped');
    $('#state-badge').className = 'badge ' + (mining ? 'on' : 'off');
    $('#hero-title').textContent = mining ? T('mine.on_title') : T('mine.ready_title');
    $('#hero-sub').textContent = mining ? T('mine.on_sub') : T('mine.ready_sub');
  }
  $('#toggle').textContent = mining ? T('mine.stop') : T('mine.start');
  $('#toggle').className = 'btn giant ' + (mining ? 'mineral' : 'primary');
  // Show BRVA mined (more motivating and correct for partial mining) instead of raw block count.
  // Each accepted block currently pays 50 BRVA on this network.
  $('#m-blocks').textContent = window.I18N.fmtNum((s.accepted || 0) * 50);
  $('#m-speed').innerHTML = fmtHashrate(mining ? (s.hashrate || 0) : 0);
  const pct = (s.cores > 0) ? Math.round((s.threads / s.cores) * 100) : 0;
  $('#m-cpu').textContent = (mining ? pct : 0) + '%';
  $('#m-session').innerHTML = fmtDuration(s.secondsMining || 0);
  $('#m-total').innerHTML = fmtDuration(s.totalSeconds || 0);
  // Real core count -> power label ("50% · 16 of 32 cores").
  if (s.cores && s.cores !== POW_CORES) { POW_CORES = s.cores; refreshPowLabel(); }
}
$('#toggle').addEventListener('click', async () => {
  if (isWaitMode()) { refreshMine(); return; } // wait mode: the button is disabled; ignore any click
  const mm = $('#mine-msg'); if (mm) mm.hidden = true; // clear any previous mining error
  if (mining) {
    await window.brisvia.stop();
  } else {
    // Read the backend's answer: if it refused to start (no peers, clock skew, syncing, wallet/worker not
    // ready...), show WHY instead of silently doing nothing (audit N2).
    const r = await window.brisvia.start(currentIntensity());
    if (r && r.error && mm) { mm.textContent = transError(r.error); mm.hidden = false; }
  }
  refreshMine();
});
// Mining power: a percentage (1..100) of the machine's cores. Named shortcuts + a slider for fine control.
// Maps legacy named settings (suave/equilibrado/intenso) to a percentage for backward compatibility.
function pctOf(v) { return ({ suave: '25', equilibrado: '50', intenso: '100' })[v] || String(parseInt(v, 10) || 50); }
function currentIntensity() { return String($('#pow-range')?.value || '50'); }
let POW_CORES = 0; // real core count, filled from miner_status
let suppressPreparingUntil = 0; // after a live power change, keep showing "Mining" instead of "Preparing…" for a few seconds
function powThreads(pct) { return POW_CORES > 0 ? Math.max(1, Math.ceil((POW_CORES * pct) / 100)) : 0; }
function refreshPowLabel() {
  const lbl = $('#pow-val'); if (!lbl) return;
  const pct = parseInt($('#pow-range')?.value || '50', 10);
  lbl.textContent = POW_CORES > 0 ? pct + '% · ' + T('mine.cores_of', { n: powThreads(pct), t: POW_CORES }) : pct + '%';
}
// Which named preset the slider value maps to (Light/Balanced/High/Max), by nearest range.
function nearestPreset(pct) { return pct <= 37 ? 25 : pct <= 62 ? 50 : pct <= 87 ? 75 : 100; }
function setPower(pct, apply) {
  pct = Math.max(1, Math.min(100, parseInt(pct, 10) || 50));
  const r = $('#pow-range'); if (r) r.value = pct;
  refreshPowLabel();
  // Auto-select the nearest named preset in BOTH controls (Mine and Settings) so they stay in sync.
  const np = nearestPreset(pct);
  $$('.mine-grid .seg-btn, #set-intensity .seg-btn').forEach((x) => x.classList.toggle('active', parseInt(x.dataset.pct, 10) === np));
  if (apply) {
    window.brisvia.setIntensity(String(pct)); // applies live (backend relaunches the engine)
    window.brisvia.settings.set('defaultIntensity', String(pct)); // remember as the default too
    try { localStorage.setItem('brv_intensity', String(pct)); } catch {} // persist across restarts (audit N7/N8)
    if (mining) suppressPreparingUntil = Date.now() + 6000; // hide the brief "Preparing…" flash during the relaunch
  }
}
$$('.mine-grid .seg-btn').forEach((b) => b.addEventListener('click', () => setPower(b.dataset.pct, true)));
{
  const r = $('#pow-range');
  if (r) {
    r.addEventListener('input', () => setPower(r.value, false)); // update label while dragging, no relaunch
    r.addEventListener('change', () => setPower(r.value, true));  // apply once when released
  }
}

// ===================== Wallet =====================
function catLabel(cat) {
  if (cat === 'generate' || cat === 'immature') return T('wallet.mined');
  if (cat === 'send') return T('wallet.sent');
  return T('wallet.received');
}
let availableBalance = 0; // spendable balance; used by the Send modal and the "use max" button
let walletEncrypted = false; // whether the Core wallet has a password (new format) vs old unencrypted one
async function loadWallet() {
  const w = await window.brisvia.wallet.summary();
  // Big number = what can be spent now (available). Maturing (mining rewards) and Incoming (unconfirmed) are
  // shown apart, each only if there is an amount. They are NEVER added into the available number.
  availableBalance = w.balance || 0;
  try { const k = await window.brisvia.wallet.kind(); walletEncrypted = !!(k && k.encrypted); } catch {}
  $('#bal-amount').textContent = fmt(w.balance);
  const mat = w.immature || 0, inc = w.incoming || 0;
  $('#bd-maturing-row').hidden = !(mat > 0);
  $('#bd-incoming-row').hidden = !(inc > 0);
  if (mat > 0) $('#bd-maturing').textContent = fmt(mat);
  if (inc > 0) $('#bd-incoming').textContent = fmt(inc);
  $('#bal-incoming-wrap').hidden = !(mat > 0 || inc > 0);
  if (!(mat > 0 || inc > 0)) $('#bal-explain').hidden = true;
  const hist = await window.brisvia.wallet.history();
  walletHistory = Array.isArray(hist) ? hist : [];
  // Keep the current page valid after a refresh (new movements can add pages, deletions can remove them).
  const pages = Math.max(1, Math.ceil(walletHistory.length / HIST_PER_PAGE));
  if (historyPage >= pages) historyPage = pages - 1;
  if (historyPage < 0) historyPage = 0;
  renderHistoryPage();
}

// ---- Movements with pagination (no long scroll: N per page + ‹ 1 2 3 › controls) ----
let walletHistory = [];
let historyPage = 0;
// 6 per page keeps the whole Wallet view scroll-free at the default window size (with the testnet banner on top)
// and keeps the Activity column about the same height as the balance + network panels on the left.
const HIST_PER_PAGE = 6;
function renderHistoryPage() {
  const list = $('#history-list'), empty = $('#history-empty'), pager = $('#history-pager');
  list.innerHTML = '';
  if (!walletHistory.length) {
    empty.hidden = false; list.hidden = true; if (pager) pager.hidden = true;
    return;
  }
  empty.hidden = true; list.hidden = false;
  const start = historyPage * HIST_PER_PAGE;
  walletHistory.slice(start, start + HIST_PER_PAGE).forEach((h) => {
    const li = document.createElement('li');
    const inc = h.amount >= 0;
    const label = h.category ? catLabel(h.category) : (inc ? T('wallet.received') : T('wallet.sent'));
    li.innerHTML = `<div><div>${label}</div><div class="muted small">${fmtDate(h.time)}</div></div>
      <div class="hist-amount ${inc ? 'in' : 'out'}">${inc ? '+' : ''}${fmt(h.amount)} BRVA</div>`;
    if (h.txid) li.addEventListener('click', () => openTxDetail(h));
    list.appendChild(li);
  });
  renderPager(pager, Math.ceil(walletHistory.length / HIST_PER_PAGE));
}
// Renders ‹ 1 2 3 › — a windowed set of page numbers around the current page (with gaps), plus prev/next arrows.
// The arrows are anchored to the edges and the numbers live in a centered block with a RESERVED (fixed) width,
// so the ‹ and › buttons never shift when the middle numbers change width between pages.
function renderPager(pager, total) {
  if (!pager) return;
  pager.innerHTML = '';
  if (total <= 1) { pager.hidden = true; return; }
  pager.hidden = false;
  const go = (p) => { historyPage = Math.max(0, Math.min(total - 1, p)); renderHistoryPage(); };
  const mkBtn = (label, page, opts = {}) => {
    const b = document.createElement('button');
    b.textContent = label;
    if (opts.active) b.classList.add('active');
    if (opts.disabled) b.disabled = true;
    if (opts.aria) b.setAttribute('aria-label', opts.aria);
    if (!opts.disabled && page != null) b.addEventListener('click', () => go(page));
    return b;
  };
  // Previous arrow (fixed left position).
  pager.appendChild(mkBtn('‹', historyPage - 1, { disabled: historyPage === 0, aria: T('wallet.prev_page') }));
  // Centered numbers block. Its width is reserved for this list's worst case (5 numbers + up to 2 gaps),
  // matching the CSS metrics (32px per number, 18px per gap, 4px flex gaps), so it never grows or shrinks
  // as the page changes and the arrows stay put.
  const nums = document.createElement('div');
  nums.className = 'pager-nums';
  const btnCount = Math.min(total, 5);
  const gapCount = total <= 5 ? 0 : (total === 6 ? 1 : 2);
  nums.style.width = (btnCount * 32 + gapCount * 18 + Math.max(0, btnCount + gapCount - 1) * 4) + 'px';
  // Window of pages: always show first and last, plus current ±1, with gaps in between.
  const set = new Set([0, total - 1, historyPage, historyPage - 1, historyPage + 1]);
  const shown = [...set].filter((n) => n >= 0 && n < total).sort((a, b) => a - b);
  let prev = -1;
  shown.forEach((n) => {
    if (n - prev > 1) { const s = document.createElement('span'); s.className = 'pager-gap'; s.textContent = '…'; nums.appendChild(s); }
    nums.appendChild(mkBtn(String(n + 1), n, { active: n === historyPage }));
    prev = n;
  });
  pager.appendChild(nums);
  // Next arrow (fixed right position).
  pager.appendChild(mkBtn('›', historyPage + 1, { disabled: historyPage === total - 1, aria: T('wallet.next_page') }));
}
function fmt(n) { return window.I18N.fmtNum(n, { maximumFractionDigits: 8, useGrouping: false }); }
function fmtDate(epoch) { return window.I18N.fmtDate(epoch); }

// Movement detail (click on the list)
async function openTxDetail(h) {
  $('#txd-rows').innerHTML = `<div class="dr"><span class="muted">${T('common.loading')}</span></div>`;
  openModal('modal-txdetail');
  let d = h;
  try { const full = await window.brisvia.wallet.txDetail(h.txid); if (full) d = { ...h, ...full }; } catch {}
  const inc = (d.amount || 0) >= 0;
  const conf = (d.confirmations != null) ? d.confirmations : 0;
  const confTxt = conf <= 0 ? T('tx.unconfirmed') : T('tx.confirmations', { n: conf, count: conf });
  const label = h.category ? catLabel(h.category) : (inc ? T('wallet.received') : T('wallet.sent'));
  $('#txd-rows').innerHTML =
    `<div class="dr"><span class="muted">${T('tx.type')}</span><span class="val">${label}</span></div>
     <div class="dr"><span class="muted">${T('tx.amount')}</span><span class="val ${inc ? 'pos' : 'neg'}">${inc ? '+' : ''}${fmt(d.amount)} BRVA</span></div>
     <div class="dr"><span class="muted">${T('tx.status')}</span><span class="val">${confTxt}</span></div>
     ${d.blockheight ? `<div class="dr"><span class="muted">${T('tx.block')}</span><span class="val">${d.blockheight}</span></div>` : ''}
     <div class="dr"><span class="muted">${T('tx.date')}</span><span class="val">${fmtDate(d.time) || '—'}</span></div>
     <div class="dr"><span class="muted">${T('tx.txid')}</span></div>
     <div class="copy-line"><code class="mono">${d.txid || '—'}</code><button class="copy-btn" id="txd-copy">${T('common.copy')}</button></div>`;
  const cp = $('#txd-copy');
  if (cp) cp.addEventListener('click', async () => { try { await navigator.clipboard.writeText(d.txid); cp.textContent = T('common.copied'); } catch {} });
}

// Balance chips (Maturing / Incoming): tapping them explains why that part can't be used yet.
$$('.bal-chip').forEach((c) => c.addEventListener('click', () => {
  const ex = $('#bal-explain');
  const key = c.classList.contains('mat') ? 'wallet.maturing_note' : 'wallet.incoming_note';
  if (!ex.hidden && ex.dataset.k === key) { ex.hidden = true; return; }
  ex.textContent = T(key); ex.dataset.k = key; ex.hidden = false;
}));

// Receive
$('#act-receive').addEventListener('click', async () => {
  const w = await window.brisvia.wallet.summary();
  showReceive(w.address);
});
$('#recv-new').addEventListener('click', async () => { const r = await window.brisvia.wallet.newAddress(); showReceive(r.address); });
$('#recv-copy').addEventListener('click', async () => {
  const addr = $('#recv-addr').textContent;
  try { await navigator.clipboard.writeText(addr); $('#recv-copy').textContent = T('receive.copied_addr'); setTimeout(() => ($('#recv-copy').textContent = T('receive.copy_addr')), 1500); } catch {}
});
// History of generated addresses (oldest first, numbered).
$('#recv-history-toggle').addEventListener('click', async () => {
  const ol = $('#recv-history');
  if (!ol.hidden) { ol.hidden = true; return; }
  const list = (await window.brisvia.wallet.addresses()) || [];
  ol.innerHTML = '';
  // Each row: the address plus the BRVA currently held at it (informational). Backend returns {address, balance};
  // older/preview shapes may return a plain string, handled here too.
  list.forEach((a) => {
    const addr = (typeof a === 'string') ? a : (a.address || '');
    const bal = (typeof a === 'string') ? null : (a.balance || 0);
    const balHtml = (bal != null) ? `<span class="addr-bal${bal > 0 ? '' : ' zero'}">${fmt(bal)} BRVA</span>` : '';
    const li = document.createElement('li');
    li.innerHTML = `<code class="mono">${addr}</code>${balHtml}`;
    ol.appendChild(li);
  });
  ol.hidden = false;
});
function showReceive(addr) {
  $('#recv-history').hidden = true;
  $('#recv-addr').textContent = addr || '';
  $('#qr').innerHTML = fakeQR(addr || '');
  openModal('modal-receive');
}
// Deterministic decorative QR (visual placeholder until the real backend generates the actual QR).
function fakeQR(seedStr) {
  const N = 21; let h = 2166136261;
  for (let i = 0; i < seedStr.length; i++) { h ^= seedStr.charCodeAt(i); h = Math.imul(h, 16777619); }
  let rects = '';
  for (let y = 0; y < N; y++) for (let x = 0; x < N; x++) {
    h ^= h << 13; h ^= h >>> 17; h ^= h << 5; h >>>= 0;
    const corner = (x < 7 && y < 7) || (x >= N - 7 && y < 7) || (x < 7 && y >= N - 7);
    const on = corner ? ((x % 6 === 0 || y % 6 === 0) || (x > 1 && x < 5 && y > 1 && y < 5) || (x >= N-6 && x <= N-2 && y > 1 && y < 5 && (x===N-6||x===N-2||y===2||y===4)) ) : (h % 100) < 46;
    if (on) rects += `<rect x="${x}" y="${y}" width="1" height="1"/>`;
  }
  return `<svg viewBox="0 0 ${N} ${N}" fill="#0B1117" shape-rendering="crispEdges">${rects}</svg>`;
}

// Send
// Accepts the amount with comma OR dot as decimal separator (an es user types "12,5", an en user "12.5").
// If there is a comma, dots are treated as thousands separators and dropped.
function parseAmount(str) {
  if (str == null) return NaN;
  let s = String(str).trim().replace(/\s/g, '');
  if (s === '') return NaN;
  if (s.includes(',')) s = s.replace(/\./g, '').replace(',', '.');
  const n = Number(s);
  return Number.isFinite(n) ? n : NaN;
}
// Plain decimal (up to 8 places, trailing zeros trimmed, dot separator) that parseAmount can read back.
function fmtAmountInput(n) {
  if (!(n > 0)) return '0';
  return n.toFixed(8).replace(/\.?0+$/, '');
}
$('#act-send').addEventListener('click', () => {
  $('#send-addr').value = ''; $('#send-amount').value = ''; $('#send-pass').value = ''; $('#send-msg').hidden = true;
  $('#send-avail').textContent = fmt(availableBalance);
  $('#send-pass-field').hidden = !walletEncrypted; // old unencrypted wallets don't ask for a password
  const go = $('#send-go'); go.disabled = false; go.classList.remove('is-busy');
  openModal('modal-send');
});
// "Use max": fills the whole available balance. The tiny network fee is taken by the node when it builds
// the transaction; on the test network it is minimal, so if it doesn't fit the user just lowers the amount.
$('#send-max').addEventListener('click', () => {
  // Leave room for the network fee so "use max" yields an amount that actually sends (audit N4):
  // fallbackfee is ~0.0001 BRVA/kB; a small reserve covers a typical transaction.
  const FEE_RESERVE = 0.0002;
  $('#send-amount').value = fmtAmountInput(Math.max(0, availableBalance - FEE_RESERVE));
  $('#send-msg').hidden = true;
});
// Clear a stale send error (invalid address/amount/password) as soon as the user edits any field.
['#send-addr', '#send-amount', '#send-pass'].forEach((s) => {
  const el = $(s);
  if (el) el.addEventListener('input', () => { const m = $('#send-msg'); if (m && !m.hidden) m.hidden = true; });
});
$('#send-go').addEventListener('click', async () => {
  const go = $('#send-go');
  if (go.disabled) return; // already sending: block a double click
  const addr = $('#send-addr').value.trim();
  const amount = parseAmount($('#send-amount').value);
  const pass = $('#send-pass').value;
  const msg = $('#send-msg'); msg.hidden = false; msg.className = 'verify-msg err';
  if (!addr || addr.length < 14 || !addr.toLowerCase().includes('brv')) { msg.textContent = T('send.invalid_addr'); return; }
  if (!(amount > 0)) { msg.textContent = T('send.invalid_amount'); return; }
  if (amount > availableBalance + 1e-8) { msg.textContent = T('send.over_balance'); return; }
  if (walletEncrypted && !pass) { msg.textContent = T('send.need_pass'); return; }
  go.disabled = true; go.classList.add('is-busy');
  const r = await window.brisvia.wallet.send(addr, amount, pass);
  if (r && r.ok) { msg.className = 'verify-msg ok'; msg.textContent = T('send.done'); setTimeout(() => closeModal('modal-send'), 1200); loadWallet(); }
  else { go.disabled = false; go.classList.remove('is-busy'); msg.textContent = r && r.error ? transError(r.error) : T('send.fail'); }
});

// ===================== Settings =====================
async function loadSettings() {
  const s = await window.brisvia.settings.get();
  $('#set-autostart').checked = !!s.autostart;
  $('#set-tray').checked = s.tray !== false;
  // Restore the chosen power from localStorage first (survives restarts even though the backend default
  // resets); fall back to the backend default. Keeps the auto-resume from mining harder than chosen (N7/N8).
  const savedPow = (() => { try { return localStorage.getItem('brv_intensity'); } catch { return null; } })();
  setPower(savedPow != null ? parseInt(savedPow, 10) : parseInt(pctOf(s.defaultIntensity), 10), false);
  $$('#set-language .seg-btn').forEach((b) => b.classList.toggle('active', b.dataset.lang === window.I18N.lang));
  applyMiningMode(s.miningMode || 'solo');
  { const _pa = $('#set-pool-addr'); if (_pa && s.poolAddress) _pa.value = s.poolAddress; }
}
$('#set-autostart').addEventListener('change', (e) => window.brisvia.settings.set('autostart', e.target.checked));
$('#set-tray').addEventListener('change', (e) => window.brisvia.settings.set('tray', e.target.checked));
$$('#set-intensity .seg-btn').forEach((b) => b.addEventListener('click', () => setPower(parseInt(b.dataset.pct, 10), true)));
// Language selector
$$('#set-language .seg-btn').forEach((b) => b.addEventListener('click', () => {
  window.I18N.setLang(b.dataset.lang);
  if (window.brisvia.setLanguage) window.brisvia.setLanguage(b.dataset.lang); // rebuilds the tray menu
}));
// Mining mode selector (solo / grouped). The Brisvia pool is being set up; until it is live, choosing "grouped"
// reveals the pool row with a "being set up" status and mining keeps running solo. When the pool is live this
// selector will point the miner at pool.brisvia.com.
let currentMiningMode = 'solo';
// Three modes: solo (mine against your own node), pool (the official Brisvia pool), custom (a third-party pool
// address the user types). Today mining runs solo until the stratum client lands; choosing pool/custom saves the
// preference and reveals the matching row, without touching the audited solo path.
function applyMiningMode(mode) {
  const m = (mode === 'pool' || mode === 'custom') ? mode : 'solo';
  currentMiningMode = m;
  $$('#set-mining-mode .seg-btn').forEach((b) => b.classList.toggle('active', b.dataset.mode === m));
  // Plain-language explanation of the chosen mode.
  const desc = $('#mining-mode-desc');
  if (desc) desc.textContent = T('settings.mode_' + m + '_desc');
  const poolRow = $('#pool-info-row');
  if (poolRow) poolRow.hidden = m !== 'pool';
  const customRow = $('#pool-custom-row');
  if (customRow) customRow.hidden = m !== 'custom';
}
$$('#set-mining-mode .seg-btn').forEach((b) => b.addEventListener('click', () => {
  applyMiningMode(b.dataset.mode);
  window.brisvia.settings.set('miningMode', b.dataset.mode);
}));
// Custom pool address: persist what the user types (on change/blur), trimmed.
{
  const poolAddrInput = $('#set-pool-addr');
  const savePoolBtn = $('#set-pool-save');
  const poolSavedTag = $('#set-pool-saved');
  const savePoolAddr = () => {
    if (!poolAddrInput) return;
    window.brisvia.settings.set('poolAddress', poolAddrInput.value.trim());
    if (poolSavedTag) { poolSavedTag.hidden = false; setTimeout(() => { poolSavedTag.hidden = true; }, 1800); }
  };
  if (poolAddrInput) poolAddrInput.addEventListener('change', savePoolAddr);
  if (savePoolBtn) savePoolBtn.addEventListener('click', savePoolAddr);
}
// Social links live in the header now (visible from any view); open them in the system browser.
$$('.hsocial').forEach((b) => b.addEventListener('click', () => window.brisvia.openUrl(b.dataset.url)));

// Security and backup
$('#set-security').addEventListener('click', () => openModal('modal-security'));

// ===================== Achievements =====================
// The 50 medals come from the wallet (they travel with the 12 words). The backend returns only ids + numbers;
// the texts are translated here via i18n. Medal styling ported from the approved preview.
const ACH_FAM_ORDER = ['blocks', 'balance', 'sends', 'receives', 'age', 'pioneer', 'rank'];
// game-icons paths (embedded inline; one icon per family, per the design spec).
const ACH_ICONS = {
  blocks: '<path fill="currentColor" d="M256 24.585L51.47 118.989L256 213.394l204.53-94.405zM38.998 133.054v258.353L247 487.415V229.063zm434.004 0L265 229.062v258.353l208.002-96.008z"/>',
  balance: '<path fill="currentColor" d="M431.1 23.53c-9.5 17.34-25.4 23.34-49.6 14.15c17.9 10.24 28.5 24.99 24.6 48.64c12.4-21.29 29.2-24.49 49.4-14.11c-18.3-11.28-33.4-24.22-24.4-48.68M206 45.39c-3.4 27.17-10.8 51.2-46.9 52.1c27.4 3.11 44.3 19.11 46.9 52.21c2.3-26.1 14.6-45.7 46.8-52.21c-34.1-4.65-48-23.18-46.8-52.1M85.7 101.2c-5.5 22-19 32.5-43.2 27.8c20.4 12.6 24.5 30.3 20.4 50.6c9-24.3 24-32.3 43.4-28c-24.4-9.4-24.2-29.2-20.6-50.4m310.4.8c3.6 21.2 3.8 41-20.5 50.4c19.3-4.3 34.3 3.7 43.3 28c-4.1-20.2 0-38 20.4-50.6c-24.2 4.7-37.7-5.8-43.2-27.8m-139.4 52c-9.6 0-18.1 2.4-23.7 5.8c-5.5 3.4-7.3 6.7-7.3 9.3s1.8 5.9 7.3 9.3c5.6 3.3 14.1 5.7 23.7 5.7c3.9 0 7.7-.4 11.1-1.1c5.5-6.1 12.5-10.2 19.7-12.6c.6-4.9-4.7-9.1-7.1-10.6c-5.6-3.4-14.1-5.8-23.7-5.8m-45.1 28.2c-6.2.9-9.1 3.1-10.2 5.4c-1.9 12.5 13 22.2 22.1 26.5c8.7 3.9 17.5 5.2 23.9 4.5s9.4-3.1 10.5-5.4c1.1-2.4.8-6.1-2.6-11.1c-12-.2-22.8-3.1-31.5-8.3c-4.9-3-9.3-6.9-12.2-11.6m98.6 2.6c-9.6 0-18.1 2.4-23.7 5.7c-5.5 3.4-7.3 6.7-7.3 9.3s1.8 5.9 7.3 9.3c5.6 3.3 14.1 5.7 23.7 5.7s18.1-2.4 23.7-5.7c5.5-3.4 7.3-6.7 7.3-9.3s-1.8-5.9-7.3-9.3c-5.6-3.3-14.1-5.7-23.7-5.7m48.8 12.3c5.1 10.4-10.3 23.8-17.6 28.4c1.4.7 3.2 1.3 5.5 1.8c6.4 1.2 15.2.6 24.2-2.7c7.7-2.8 14.1-7 18.4-11.3c.4-5.7 1.2-11 4.7-15c-10.5-6.9-24.8-5.1-35.2-1.2m-202-1.5c-9.6 0-18.1 2.4-23.7 5.7c4.6 6.3 5.7 13.2 4.5 20.8c5.2 2.1 11.9 3.5 19.2 3.5c9.6 0 18.1-2.4 23.7-5.7c5.5-3.4 7.3-6.7 7.3-9.3s-1.8-5.9-7.3-9.3c-5.6-3.3-14.1-5.7-23.7-5.7m251.1 14.2c-2.7 12.2 11.8 23 20.5 27.7c8.5 4.4 17.1 6.2 23.6 5.9c6.4-.4 9.6-2.5 10.8-4.8s1.2-6.1-2.2-11.6s-9.8-11.6-18.3-16.1c-6.5-3.1-28.9-11.1-34.4-1.1m-302.5-.9c-5.9-.1-13.1 1.2-20.3 4.2c-8.8 3.7-15.7 9.2-19.5 14.4c-3.8 5.3-4.2 9-3.2 11.4s4 4.8 10.4 5.8c6.3.9 15.1-.2 24-4c8.9-3.7 15.8-9.2 19.6-14.4c3.8-5.3 4.2-9 3.2-11.4c-4.3-4.9-8.5-6-14.2-6m168 13.1c-3.1 5.8-8.3 9.8-14.4 12.1c6.4 3.9 11.5 9.7 13.1 17.2c2.2 10.5-3 20.4-10.7 27.5c-7.7 7.2-18.2 12.4-30.5 14.9c-12.2 2.6-24 2.1-33.9-1.3s-18.6-10.4-20.8-20.8c-2.2-10.5 2.9-20.4 10.6-27.5c7.1-6.6 16.7-11.6 27.7-14.3c-4.4-2-8.4-4.4-12-7.1c-2.9 5.2-7.5 9.4-12.8 12.6c-9 5.4-20.4 8.3-32.9 8.3c-9.9 0-19.1-1.8-27-5.3c-6.1 7.9-15.2 14.5-26 19c-10.5 4.4-21 6.3-30.6 5.5c-3.8 7.5-11.4 12.4-19.6 15c-10.1 3.1-21.9 3.2-34 .2c-.3-.1-.6-.1-.8-.2V324c2.8-1.5 5.9-2.6 9-3.3c3.4-.8 7-1.2 10.7-1.3v-.2c-2.9-10.3 1.7-20.5 8.9-28.1s17.4-13.5 29.5-16.8c11.56-3 23.1-3.7 33.9-.8c10.1 2.8 19.3 9.2 22.1 19.5c2.9 10.3-1.6 20.5-8.9 28.1c-7.2 7.6-17.4 13.4-29.5 16.8c-1.8.5-3.7.9-5.6 1.3c7.46 8.4 11.8 21.7 9.3 30.2c-3.3 10.1-12.7 16.1-22.9 18.5c-10.3 2.4-22 1.6-33.9-2.2c-8.7-2.8-16.4-6.9-22.6-12.1v113.9h77.2c-4-10.7 3.9-11.4-7.2-16.1c-11.6-4.7-21-11.8-27.3-20.2s-9.5-19.1-5.5-29c4.1-9.8 13.9-15.1 24.3-16.7c10.4-1.5 22.1.1 33.6 4.9c11.6 4.7 21 11.8 27.3 20.2c2.3 3.1 4.2 6.4 5.5 9.9c8.4-.6 16.4.4 23.5 2.8c9.9 3.4 18.6 10.4 20.8 20.8c1.8 8.6-1.3 16.8-6.7 23.4h256.1c-6.3-7.3-10-16.6-7.4-26.2c2.8-10.3 11.9-16.7 22-19.6s21.9-2.7 33.9.6c1.7.5 3.3 1 4.9 1.6V342.7c-11 1.8-21.6 1.1-30.6-2c-9.9-3.4-18.6-10.4-20.8-20.8c-2.2-10.5 2.9-20.4 10.6-27.5c13.2-10.1 25.8-15.4 40.8-16.3V275c-15 .2-35.9-5.5-44.9-13.6c-9.3-.2-19.2-2.9-28.9-8c-10.7-5.6-19.1-13.1-24.6-21.7c-5.3 4-11.5 7.3-18.3 9.8c-11.8 4.3-23.5 5.5-33.8 3.5c-8.6-1.7-16.7-5.9-21.3-13.1c-3.8.6-7.9.9-12 .9c-12.5 0-23.9-2.9-32.9-8.3c-1.3-.8-2.5-1.6-3.7-2.5m207 5.9c1.9 11.1.3 19.9-8 26.7c7.8 2.2 14.2 2.9 21.4 2.4v-29.6c-4.5-.3-9.3-.2-13.4.5M18 229.7v28.9c9.44 3.2 21.18 4.7 30.4 2c3.3-1 5.5-2.2 7-3.6c-7.21-5.3-11.24-12.3-11.3-20.7c-3.7-2-8-3.7-12.8-4.9c-4.6-1.1-9.2-1.7-13.3-1.7m203.2 17.1c-9.4 2-17.2 6.1-22 10.4c-4.7 4.5-5.7 8.1-5.2 10.7c.5 2.5 3 5.4 9.1 7.5s14.9 2.7 24.3.7s17.2-6.1 22-10.4c4.7-4.5 5.8-8.1 5.3-10.6c-.6-2.6-3-5.4-9.1-7.6c-8.3-2.4-16.6-2.4-24.4-.7M81.9 291.6c-9.3 2.6-16.9 7.2-21.3 11.8c-4.4 4.8-5.3 8.4-4.6 11c.7 2.5 3.3 5.1 9.6 7c6.1 1.7 15 1.7 24.3-.9c9.2-2.5 16.9-7.1 21.3-11.8s5.3-8.4 4.6-10.9s-3.4-5.2-9.6-7c-7.91-2-16.83-1.3-24.3.8m261 .1c6.5.1 12.6 1.1 18.2 3c10 3.5 18.7 10.4 20.9 20.9c1 4.9.4 9.8-1.4 14.3c9.9 3.4 18.3 8.6 24.6 15c7.2 7.6 11.8 17.8 9 28.1s-11.9 16.8-22 19.6c-10.1 2.9-21.8 2.7-33.9-.6c-12.1-3.2-22.3-9-29.6-16.6c-4.4-4.6-7.9-10.1-9.2-16.1c-4.4-.4-8.7-1.3-12.6-2.7c-9.9-3.4-18.6-10.3-20.8-20.8s3-20.3 10.7-27.5c7.6-7.1 18.2-12.3 30.4-14.9c5.4-1.1 10.6-1.7 15.7-1.7m151.1 2.4c-2.2.2-4.4.5-6.6 1c-9.4 2-17.2 6.1-22 10.4c-4.7 4.5-5.7 8.1-5.2 10.7c.5 2.5 3 5.4 9 7.5c6.2 2.1 15 2.7 24.4.7c.1 0 .3-.1.4-.1zM330.9 311c-9.3 2-17.2 6.1-22 10.5c-4.6 4.4-5.7 8-5.2 10.6c.5 2.5 3 5.4 9.1 7.6c6.1 2.1 14.9 2.7 24.3.7s17.3-6.1 22-10.5c4.7-4.5 5.8-8.1 5.3-10.6c-.6-2.5-3-5.4-9.1-7.6c-8.3-2.4-16.6-2.4-24.4-.7m-133.4 5.7c12.2 2.6 22.7 7.9 30.4 15.1c7.6 7.2 12.8 17.1 10.5 27.5c-2.3 10.5-11 17.4-21 20.8c-9.9 3.3-21.7 3.8-33.9 1.1c-12.2-2.6-22.7-7.9-30.4-15c-7.6-7.2-12.8-17.1-10.5-27.6s11-17.4 21-20.7c11.5-3.6 23-3.4 33.9-1.2m-28.2 18.2c-6.1 2.1-8.6 5-9.1 7.5c-.6 2.6.5 6.2 5.2 10.6c4.7 4.5 12.5 8.6 21.9 10.6c9.4 2.1 18.2 1.5 24.4-.5c6.1-2.2 8.5-5 9.1-7.6c.5-2.5-.5-6.1-5.2-10.6c-4.8-4.4-12.6-8.6-21.9-10.6c-8-1.5-16.7-1.8-24.4.6M32 338.2c-6.2 1.5-9 4.1-9.8 6.6s-.1 6.1 4.1 11.1c4.3 4.8 11.7 9.7 20.8 12.7c9.2 2.9 18 3.3 24.3 1.8s9-4.1 9.8-6.6c.8-2.4.1-6.1-4.1-11.1c-4.3-4.8-11.6-9.7-20.8-12.7c-8.27-2.3-16.36-3.4-24.3-1.8m336.9 7c-10.3 6.9-20.1 11.5-30.6 13.3c5.3 8.5 16.8 14 24.7 16.2c9.3 2.5 18.1 2.4 24.4.7c6.2-1.9 8.8-4.6 9.5-7.1c.6-2.5-.2-6.2-4.7-10.9c-7.7-6.2-15.2-10.4-23.3-12.2m-53.7 34.9c9.9 3.4 18.6 10.3 20.8 20.8s-3 20.4-10.7 27.5c-7.6 7.1-18.2 12.3-30.4 14.9c-12.3 2.6-24 2.1-33.9-1.3c-10-3.5-18.7-10.4-20.9-20.9c-2.2-10.4 3-20.3 10.7-27.5c7.7-7.1 18.2-12.3 30.4-14.9c11.1-2.3 23.6-2.2 34 1.4M285 396.3c-9.4 2-17.3 6.1-22 10.5c-4.7 4.5-5.8 8.1-5.3 10.6c.6 2.6 3 5.4 9.1 7.6c6.1 2.1 15 2.7 24.4.7c9.3-2 17.2-6.1 22-10.5c4.6-4.4 5.7-8 5.2-10.6c-.5-2.5-3-5.4-9.1-7.6c-8.1-2.3-16.3-2.3-24.3-.7m-201.8 27c-6.4 1-9.3 3.4-10.3 5.8s-.6 6.1 3.2 11.4c3.9 5.2 10.9 10.6 19.7 14.3c9 3.6 17.7 4.6 24.2 3.7c6.3-1 9.3-3.4 10.3-5.8c.9-2.4.5-6.1-3.3-11.4c-3.9-5.2-10.8-10.6-19.7-14.3c-8.15-2.8-16.13-4.7-24.1-3.7m387.9 34.5c-6.5.1-18.5 1-20.5 8.2c-.1 12.5 16 19.8 25.6 22.5H494v-25.3c-7.3-3.4-15.2-5.2-22.9-5.4m-323.8.8c-4.3 9.9-16.3 16.3-24.7 17.7c-3 .4-6.1.6-9.3.5c-1.9 6.1 5.6 10.3 9.7 11.7h25.2c8.6-2 15.7-6 20.1-10c4.7-4.5 5.9-8.1 5.3-10.6c-.5-2.6-3-5.4-9.1-7.6c-6.2-1.6-11.4-2.4-17.2-1.7"/>',
  sends: '<path fill="currentColor" d="M298.9 24.31c-14.9.3-25.6 3.2-32.7 8.4l-97.3 52.1l-54.1 73.59c-11.4 17.6-3.3 51.6 32.3 29.8l39-51.4c49.5-42.69 150.5-23.1 102.6 62.6c-23.5 49.6-12.5 73.8 17.8 84l13.8-46.4c23.9-53.8 68.5-63.5 66.7-106.9l107.2 7.7l-1-112.09zM244.8 127.7c-17.4-.3-34.5 6.9-46.9 17.3l-39.1 51.4c10.7 8.5 21.5 3.9 32.2-6.4c12.6 6.4 22.4-3.5 30.4-23.3c3.3-13.5 8.2-23 23.4-39m-79.6 96c-.4 0-.9 0-1.3.1c-3.3.7-7.2 4.2-9.8 12.2c-2.7 8-3.3 19.4-.9 31.6c2.4 12.1 7.4 22.4 13 28.8c5.4 6.3 10.4 8.1 13.7 7.4c3.4-.6 7.2-4.2 9.8-12.1c2.7-8 3.4-19.5 1-31.6c-2.5-12.2-7.5-22.5-13-28.8c-4.8-5.6-9.2-7.6-12.5-7.6m82.6 106.8c-7.9.1-17.8 2.6-27.5 7.3c-11.1 5.5-19.8 13.1-24.5 20.1c-4.7 6.9-5.1 12.1-3.6 15.2c1.5 3 5.9 5.9 14.3 6.3c8.4.5 19.7-1.8 30.8-7.3s19.8-13 24.5-20c4.7-6.9 5.1-12.2 3.6-15.2c-1.5-3.1-5.9-5.9-14.3-6.3c-1.1-.1-2.1-.1-3.3-.1m-97.6 95.6c-4.7.1-9 .8-12.8 1.9c-8.5 2.5-13.4 7-15 12.3c-1.7 5.4 0 11.8 5.7 18.7c5.8 6.8 15.5 13.3 27.5 16.9c11.9 3.6 23.5 3.5 32.1.9c8.6-2.5 13.5-7 15.1-12.3c1.6-5.4 0-11.8-5.8-18.7c-5.7-6.8-15.4-13.3-27.4-16.9c-6.8-2-13.4-2.9-19.4-2.8"/>',
  receives: '<path fill="currentColor" d="M258 21.89c-.5 0-1.2 0-1.8.12c-4.6.85-10.1 5.1-13.7 14.81c-3.8 9.7-4.6 23.53-1.3 38.34c3.4 14.63 10.4 27.24 18.2 34.94c7.6 7.7 14.5 9.8 19.1 9c4.8-.7 10.1-5.1 13.7-14.7c3.8-9.64 4.8-23.66 1.4-38.35c-3.5-14.8-10.4-27.29-18.2-34.94c-6.6-6.8-12.7-9.22-17.4-9.22M373.4 151.4c-11 .3-24.9 3.2-38.4 8.9c-15.6 6.8-27.6 15.9-34.2 24.5c-6.6 8.3-7.2 14.6-5.1 18.3c2.2 3.7 8.3 7.2 20 7.7c11.7.7 27.5-2.2 43-8.8c15.5-6.7 27.7-15.9 34.3-24.3c6.6-8.3 7.1-14.8 5-18.5c-2.1-3.8-8.3-7.1-20-7.5c-1.6-.3-3-.3-4.6-.3m-136.3 92.9c-6.6.1-12.6.9-18 2.3c-11.8 3-18.6 8.4-20.8 14.9c-2.5 6.5 0 14.3 7.8 22.7c8.2 8.2 21.7 16.1 38.5 20.5c16.7 4.4 32.8 4.3 44.8 1.1c12.1-3.1 18.9-8.6 21.1-15c2.3-6.5 0-14.2-8.1-22.7c-7.9-8.2-21.4-16.1-38.2-20.4c-9.5-2.5-18.8-3.5-27.1-3.4m160.7 58.1L336 331.7c4.2.2 14.7.5 14.7.5l6.6 8.7l54.7-28.5zm-54.5.1l-57.4 27.2c5.5.3 18.5.5 23.7.8l49.8-23.6zm92.6 10.8l-70.5 37.4l14.5 18.7l74.5-44.6zm-278.8 9.1a40.3 40.3 0 0 0-9 1c-71.5 16.5-113.7 17.9-126.2 17.9H18v107.5s11.6-1.7 30.9-1.8c37.3 0 103 6.4 167 43.8c3.4 2.1 10.7 2.9 19.8 2.9c24.3 0 61.2-5.8 69.7-9C391 452.6 494 364.5 494 364.5l-32.5-28.4s-79.8 50.9-89.9 55.8c-91.1 44.7-164.9 16.8-164.9 16.8s119.9 3 158.4-27.3l-22.6-34s-82.8-2.3-112.3-6.2c-15.4-2-48.7-18.8-73.1-18.8"/>',
  age: '<path fill="currentColor" d="M92.656 19.188v41.5h331.72v-41.5zM119.5 79.374V433.53h22.28V79.376H119.5zm46.594 0c3.212 43.324 13.312 82.022 27.78 110.906c17.685 35.304 40.845 54.75 64.064 54.75s46.346-19.446 64.03-54.75c14.47-28.883 24.57-67.58 27.782-110.905H166.094zm209.156 0V433.53h22.28V79.376h-22.28zm-117.313 185.22c-23.218 0-46.378 19.415-64.062 54.717c-14.835 29.614-25.098 69.562-28.03 114.22H350c-2.933-44.658-13.197-84.606-28.03-114.22c-17.686-35.302-40.814-54.718-64.033-54.718zM92.657 452.218v41.467h331.718V452.22H92.655z"/>',
  pioneer: '<path fill="currentColor" d="m375.7 20.11l-15.6 3.53c5.5 24.18 10.9 48.4 16.4 72.61c-12.4-1.91-22.7-3.61-34-5.36l6.5 28.91c12.4 1.6 22.6 3.6 34 5.3l7.6 33.6c9.4 41.6 18.9 83.3 28.3 124.9c-12.4-1.9-22.6-3.7-34-5.4l6.5 28.8c12.3 2.1 22.7 3.4 34 5.4c13.6 59.8 27 119.7 40.6 179.5l15.6-3.7c-37.4-162.5-73.8-328.9-105.9-468.09M391.4 307c-12.9-1.9-23.9-3.4-33.7-4l7.4 32.9h.4c12.2 1.3 22.5 3.1 33.5 4.7zm-33.7-4l-6.7-29.5c-14.4-1.5-24.2-1.5-32.7.3l7 31.3c10.4-2.4 20.6-2.9 32.4-2.1m-32.4 2.1c-10.3 2.4-19.7 6.3-30.1 12l7.4 32.7c9.8-5.2 20.1-11.2 29.8-13.4zm-30.1 12l-6.6-29.5c-7.8 4.8-17.2 11.1-28.6 18.8l6.5 28.9c10.8-7.4 20.2-13.4 28.7-18.2m-28.7 18.2c-10.3 7-18.9 13-28.4 19.5l7.6 33.2c10-7.2 18.8-13.1 28.3-19.6zm-28.4 19.5l-6.5-28.9c-10.8 7.4-20.1 13.4-28.7 18.2l6.7 29.5c7.8-4.8 17.2-11.1 28.5-18.8m-28.5 18.8c-12.3 7.5-21.2 11.7-29.7 13.7l7 31.2c10.4-2.4 19.8-6.4 30.1-12.1zm-29.7 13.7l-7.1-31.2c-10.3 2.3-20.5 2.8-32.3 2.1l6.7 29.5c14.3 1.5 24.1 1.5 32.7-.4m-32.7.4c-9.1-.9-20.3-2.6-33.9-4.7l7.6 33.6s16 2.9 33.7 4zm-33.9-4.7l-6.5-28.8c-12.35-2-22.71-3.4-34.02-5.4l6.53 28.8c12.36 1.8 22.69 3.8 33.99 5.4m-6.5-28.8c12.9 1.9 23.9 3.4 33.7 4l-7.5-32.9c-9.1-1-20.2-2.6-33.8-4.7zm-7.6-33.6l-6.52-28.9c-12.39-1.8-22.66-3.7-34.02-5.3l6.52 28.8c12.35 2 22.71 3.4 34.02 5.4m-6.52-28.9c12.82 2 23.92 3.5 33.72 4.1l-7.5-32.9c-9.07-.9-20.22-2.5-33.84-4.7zm-7.6-33.6l-6.52-28.8c-12.33-2.1-22.71-3.3-34.02-5.3l6.52 28.9c12.36 1.9 22.66 3.6 34.02 5.2m-6.52-28.8c12.89 2 23.93 3.5 33.72 4l-7.45-32.9c-11.72-2.1-24.9-3.3-33.87-4.7zm33.72 4l6.64 29.5c14.4 1.6 24.2 1.5 32.7-.4l-7-31.2c-10.4 2.4-20.6 2.9-32.34 2.1m32.24-2.1c10.4-2.3 19.8-6.3 30.2-12l-7.5-32.9c-12.3 7.5-21.2 11.7-29.7 13.7zm37.2 19.2l6.6 29.5c7.8-4.8 17.2-11 28.6-18.8l-6.6-28.8c-10.7 7.3-20.1 13.4-28.6 18.1m28.6-18.1c10.3-7 18.9-13.1 28.5-19.4l-7.6-33.66c-10.4 7.05-19 13.01-28.5 19.56zm28.5-19.4l6.5 28.7c10.8-7.3 20.1-13.4 28.7-18.1l-6.7-29.5c-7.8 4.8-17.2 11.1-28.5 18.9m28.5-18.9c12.3-7.55 21.2-11.74 29.7-13.68l-7-31.2c-11.1 3-21.8 7.36-30.1 11.95zm29.7-13.68l7.1 31.28c10.3-2.4 20.5-2.9 32.3-2.2l-6.7-29.53c-14.3-1.51-24.1-1.48-32.7.45m32.7-.45c9.1.97 20.3 2.59 33.9 4.72l-7.6-33.59s-16.1-2.91-33.7-4.03zm6.7 29.53l7.4 32.8c9.2 1 20.3 2.6 33.9 4.8l-7.6-33.5c-12.9-2-23.9-3.5-33.7-4.1m41.3 37.6l6.5 28.8c12.4 1.9 22.7 3.7 34.1 5.3l-6.6-28.8c-12.4-1.9-22.7-3.7-34-5.3m6.5 28.8c-12.8-2-23.9-3.5-33.7-4l7.5 33c9.1.9 20.2 2.5 33.8 4.6zm7.6 33.6l6.6 28.9c12.4 2 22.7 3.4 34 5.3l-6.5-28.9c-12.4-1.8-22.7-3.7-34.1-5.3m6.6 28.9c-12.9-2-24-3.5-33.8-4l7.5 32.9c9.1.8 20.2 2.6 33.9 4.7zm-33.8-4l-6.6-29.5c-14.4-1.6-24.2-1.5-32.7.4l7 31.1c10.3-2.3 20.6-2.8 32.3-2m-32.3 2c-10.3 2.5-19.8 6.4-30.1 12l7.5 33c12.3-7.5 21.1-11.8 29.7-13.8zm-30.1 12l-6.7-29.5c-7.8 4.9-17.1 11-28.5 18.9l6.5 28.8c10.8-7.3 20.1-13.5 28.7-18.2m-28.7 18.2c-10.5 6.9-18.7 13.2-28.4 19.5l7.6 33.6c10.4-7 19-13 28.4-19.5zM224 292.2l-6.5-28.8c-10.8 7.3-20.1 13.4-28.7 18.2l6.7 29.5c7.8-4.8 17.1-11.1 28.5-18.9m-28.5 18.9c-12.3 7.5-21.2 11.7-29.7 13.6l7 31.4c10.3-2.4 19.8-6.4 30.1-12zm-29.7 13.6l-7.1-31.1c-10.3 2.3-20.5 2.8-32.2 2.1l6.5 29.5c14.4 1.5 24.2 1.5 32.8-.5m-7.1-31.1c10.3-2.4 19.8-6.2 30.1-11.9l-7.4-33.1c-12.3 7.7-21.2 11.9-29.8 13.7zm-7.1-31.3l-7-31.2c-10.3 2.4-20.5 3-32.2 2.2l6.6 29.5c14.3 1.5 24.1 1.5 32.6-.5m-7-31.2c10.3-2.3 19.7-6.3 30.1-12l-7.5-32.9c-12.3 7.6-21.1 11.9-29.7 13.7zm30.1-12l6.7 29.5c7.8-4.6 17.1-11 28.5-18.8l-6.5-28.8c-10.8 7.3-20.1 13.4-28.7 18.1m28.7-18c10.2-7.2 18.9-13 28.4-19.5l-7.6-33.7c-10.3 7.2-19 13.1-28.4 19.6zm28.4-19.5l6.5 28.8c10.8-7.3 20.1-13.4 28.7-18.1l-6.7-29.5c-7.8 4.7-17.1 11-28.5 18.8m28.5-18.9c12.3-7.6 21.2-11.8 29.7-13.6l-7-31.2c-10.3 2.2-19.8 6.1-30.1 11.8zm29.7-13.6l7.1 31.1c10.3-2.3 20.5-2.9 32.3-2.1l-6.7-29.5c-14.3-1.6-24.1-1.5-32.7.5m7.1 31.1c-10.3 2.4-19.8 6.4-30.1 12l7.4 32.9c12.3-7.5 21.2-11.8 29.8-13.6zm-58.8 30.1c-10.3 7.1-19 13-28.4 19.5l7.6 33.7c10.3-7.2 18.9-13 28.4-19.5z"/>',
  rank: '<path fill="currentColor" d="M140.5 19.156V192.28l21.813 28.532h15.53V19.156zm56.03 0v201.656h122.064V19.156zm140.75 0v201.656h12.345l22.094-28.53V19.155zM173.94 239.5v18.125h164.09V239.5zm30.78 36.813l8.032 10.53c-25.262 12.014-45.128 33.46-55.094 59.813l65.03 47.47l5.47 3.968l-2.094 6.437l-17.312 53.69l45.656-33.064l5.5-3.97l5.47 3.97l62.468 45.22c24.872-19.957 40.78-50.6 40.78-85.063c0-6.494-.573-12.854-1.655-19.032l-58.845 42.94l-11-15.095L361.688 347c-10.683-28.55-32.932-51.392-61.125-62.78l6.125-7.908h-38.813l-25.5 78l-17.75-5.812l23.594-72.188h-43.5zm-52.374 89.625a110 110 0 0 0-1.72 19.375c0 32.163 13.84 61.008 35.907 80.937l19.69-61.03l-53.876-39.283zm107.562 78.343l-51.53 37.314c15.266 8.124 32.707 12.72 51.25 12.72c18.673-.002 36.218-4.676 51.562-12.908l-51.282-37.125z"/>',
};
const ACH_LOCK = '<svg class="ach-lock" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 1a5 5 0 0 0-5 5v3H6a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-9a2 2 0 0 0-2-2h-1V6a5 5 0 0 0-5-5zm3 8H9V6a3 3 0 0 1 6 0z"/></svg>';
function achIcon(fam) { return `<svg class="ach-ico" viewBox="0 0 512 512" xmlns="http://www.w3.org/2000/svg">${ACH_ICONS[fam] || ''}</svg>`; }
// Progress text ("current / threshold") for the count families; the rest show no number (milestone/combo).
function achProgress(a) {
  if (a.family === 'blocks' || a.family === 'sends' || a.family === 'receives') {
    return window.I18N.fmtNum(Math.min(a.current, a.threshold)) + ' / ' + window.I18N.fmtNum(a.threshold);
  }
  if (a.family === 'balance') {
    return window.I18N.fmtNum(Math.min(Math.floor(a.current), a.threshold)) + ' / ' + window.I18N.fmtNum(a.threshold) + ' BRVA';
  }
  return '';
}
let achData = null;
async function fetchAchievements() {
  let r = null;
  try { r = window.brisvia.achievements ? await window.brisvia.achievements() : null; } catch {}
  if (!r || !Array.isArray(r.list)) return null;
  achData = r;
  if (Array.isArray(r.justUnlocked)) r.justUnlocked.forEach((id) => showAchievementToast(id));
  const av = document.querySelector('.view[data-view="achievements"]');
  if (av && !av.hidden) renderAchievements(r.list);
  return r;
}
async function loadAchievements() {
  if (achData) renderAchievements(achData.list); // paint the cached state immediately, then refresh
  const r = await fetchAchievements();
  if (!r && !achData) renderAchievements(null);
}
function renderAchievements(list) {
  const wrap = $('#ach-families'), empty = $('#ach-empty');
  if (!list || !list.length) {
    wrap.innerHTML = ''; wrap.hidden = true; if (empty) empty.hidden = false;
    return;
  }
  wrap.hidden = false; if (empty) empty.hidden = true;
  const byFam = {};
  list.forEach((a) => { (byFam[a.family] = byFam[a.family] || []).push(a); });
  wrap.innerHTML = '';
  ACH_FAM_ORDER.forEach((fam) => {
    const items = byFam[fam];
    if (!items) return;
    const unlocked = items.filter((a) => a.unlocked).length;
    const row = document.createElement('div');
    row.className = 'ach-fam';
    const info = document.createElement('div');
    info.className = 'ach-fam-info';
    info.innerHTML = `<span class="ach-fam-icon">${achIcon(fam)}</span>
      <span class="ach-fam-text">
        <span class="ach-fam-name">${T('ach.fam.' + fam)}</span>
        <span class="ach-fam-count">${window.I18N.fmtNum(unlocked)} / ${window.I18N.fmtNum(items.length)}</span>
      </span>`;
    const strip = document.createElement('div');
    strip.className = 'ach-medals';
    items.forEach((a) => {
      const tile = document.createElement('div');
      tile.className = 'ach-tile ' + (a.unlocked ? 'unlocked' : 'locked');
      tile.tabIndex = 0;
      tile.dataset.name = T('ach.names.' + a.id);
      tile.dataset.desc = T('ach.descs.' + a.id);
      tile.dataset.prog = achProgress(a);
      tile.innerHTML = `<div class="ach-medal t-${a.tier}">${achIcon(fam)}${ACH_LOCK}</div>`;
      strip.appendChild(tile);
    });
    row.appendChild(info);
    row.appendChild(strip);
    wrap.appendChild(row);
  });
}
// Floating tooltip for medals (body-level, so the horizontal medal strip never clips it).
(function initAchTooltip() {
  let el = null, timer = null;
  function box() { if (!el) { el = document.createElement('div'); el.className = 'ach-tooltip'; document.body.appendChild(el); } return el; }
  function show(t) {
    const name = t.dataset.name || '', desc = t.dataset.desc || '', prog = t.dataset.prog || '';
    const locked = t.classList.contains('locked');
    const b = box();
    b.innerHTML = `<strong>${name}</strong>${desc ? `<span>${desc}</span>` : ''}${prog ? `<span class="ach-tt-prog">${prog}</span>` : ''}${locked ? `<span class="ach-tt-locked">${T('ach.locked')}</span>` : ''}`;
    b.classList.add('on');
    const r = t.getBoundingClientRect();
    let left = r.left + r.width / 2 - b.offsetWidth / 2;
    left = Math.max(8, Math.min(left, window.innerWidth - 8 - b.offsetWidth));
    let top = r.bottom + 8;
    if (top + b.offsetHeight > window.innerHeight - 8) top = r.top - b.offsetHeight - 8;
    b.style.left = left + 'px';
    b.style.top = top + 'px';
  }
  function hide() { if (timer) { clearTimeout(timer); timer = null; } if (el) el.classList.remove('on'); }
  // Treat the whole medal tile as ONE surface: moving between its inner parts (drawing, border) does NOT
  // restart or hide the tooltip. It only restarts when entering ANOTHER medal, and only hides when the mouse
  // actually leaves the medal. (mouseover/mouseout bubble between children; hence the filter.)
  let curTile = null;
  document.addEventListener('mouseover', (e) => {
    const t = e.target.closest('.ach-tile'); if (!t) return;
    if (t === curTile) return; // already inside this medal: don't restart
    curTile = t; hide(); timer = setTimeout(() => show(t), 120);
  });
  document.addEventListener('mouseout', (e) => {
    const t = e.target.closest('.ach-tile'); if (!t) return;
    if (e.relatedTarget && t.contains(e.relatedTarget)) return; // moved to another part of the SAME medal
    curTile = null; hide();
  });
  document.addEventListener('focusin', (e) => { const t = e.target.closest('.ach-tile'); if (t) show(t); });
  document.addEventListener('focusout', hide);
  document.addEventListener('click', hide);
  window.addEventListener('scroll', hide, true);
})();
// Sober toast when a medal unlocks (no emoji, no alert).
function achToastContainer() {
  let c = document.getElementById('ach-toasts');
  if (!c) { c = document.createElement('div'); c.id = 'ach-toasts'; c.className = 'ach-toasts'; document.body.appendChild(c); }
  return c;
}
function showAchievementToast(id) {
  const name = T('ach.names.' + id);
  const c = achToastContainer();
  const el = document.createElement('div');
  el.className = 'ach-toast';
  el.innerHTML = `<span class="ach-toast-tag">${T('ach.toast')}</span><span class="ach-toast-name">${name}</span>`;
  c.appendChild(el);
  requestAnimationFrame(() => el.classList.add('show'));
  setTimeout(() => { el.classList.remove('show'); setTimeout(() => el.remove(), 400); }, 4500);
}

// ===================== Updates (self-updater) =====================
// On startup (and every 6 h) it checks for a newer signed version; if there is one, it shows a pop-up with a button
// that downloads it, verifies its signature, installs and restarts. It can also be checked manually from Settings.
let updatePendingVersion = null;
async function checkForUpdate(manual) {
  const btn = $('#set-update');
  if (manual && btn) { btn.disabled = true; btn.textContent = T('update.checking'); }
  let res = null;
  try { res = window.brisvia.checkUpdate ? await window.brisvia.checkUpdate() : null; } catch {}
  if (res && res.available) {
    updatePendingVersion = res.version;
    const cur = ($('#ver-label')?.textContent || '').trim();
    if ($('#upd-ver')) $('#upd-ver').textContent = T('update.version_line', { v: res.version, cur });
    let dismissed = null; try { dismissed = localStorage.getItem('brv_update_dismissed'); } catch {}
    // On the automatic check, don't nag again if the user already chose "Later" for THIS version.
    if (!manual && dismissed === res.version) { if (btn) { btn.disabled = false; btn.textContent = T('update.check'); } return; }
    openModal('modal-update'); // pop-up: OK installs, Later dismisses
    if (btn) { btn.disabled = false; btn.textContent = T('update.check'); }
  } else if (manual && btn) {
    btn.disabled = false;
    btn.textContent = res ? T('update.none') : T('update.error');
    setTimeout(() => { btn.textContent = T('update.check'); }, 4000);
  }
}
async function installUpdate() {
  const b = $('#upd-ok');
  if (b) { b.disabled = true; b.textContent = T('update.installing'); }
  // If mining now, remember it so mining resumes automatically after the app restarts with the new version.
  try { if (mining) localStorage.setItem('brv_resume_mining', '1'); } catch {}
  try { await window.brisvia.installUpdate(); } // downloads, verifies the signature, installs and restarts
  catch { if (b) { b.disabled = false; b.textContent = T('update.install_now'); } }
}
if ($('#set-update')) $('#set-update').addEventListener('click', () => checkForUpdate(true));
if ($('#upd-ok')) $('#upd-ok').addEventListener('click', installUpdate);
if ($('#upd-later')) $('#upd-later').addEventListener('click', () => {
  try { if (updatePendingVersion) localStorage.setItem('brv_update_dismissed', updatePendingVersion); } catch {}
  closeModal('modal-update');
});

// The tempting "view my 12 words" button was removed on purpose: the recovery phrase is shown ONLY once,
// when the wallet is created (with a mandatory backup verification). To re-check a backup afterwards the user
// uses "Verify my backup" below, which compares what they type WITHOUT ever revealing the phrase again.
// (A protected "show recovery phrase" behind a wallet password is planned together with wallet encryption for mainnet.)
// Verify backup: the user types their 12 words and they are compared against the wallet's.
$('#sec-verify').addEventListener('click', () => {
  const grid = $('#vb-grid'); grid.innerHTML = '';
  for (let i = 0; i < 12; i++) {
    const li = document.createElement('li');
    const inp = document.createElement('input');
    inp.type = 'text'; inp.autocomplete = 'off'; inp.spellcheck = false; inp.setAttribute('aria-label', `${i + 1}`);
    li.appendChild(inp); grid.appendChild(li);
  }
  $('#vb-msg').hidden = true;
  closeModal('modal-security'); openModal('modal-verify-backup');
});
$('#vb-check').addEventListener('click', async () => {
  const words = [...$('#vb-grid').querySelectorAll('input')].map((i) => i.value.trim().toLowerCase()).filter(Boolean);
  const msg = $('#vb-msg'); msg.hidden = false;
  if (words.length !== 12) { msg.className = 'verify-msg err'; msg.textContent = T('security.verify_len', { n: words.length }); return; }
  // Verify by fingerprint (no password, never reveals the stored phrase). Falls back to getSeed in the browser mock.
  let ok = false;
  if (window.brisvia.wallet.checkBackup) {
    const r = await window.brisvia.wallet.checkBackup(words);
    ok = !!(r && r.ok);
  } else {
    const real = await window.brisvia.wallet.getSeed();
    ok = Array.isArray(real) && real.length === 12 && real.every((w, i) => w === words[i]);
  }
  msg.className = ok ? 'verify-msg ok' : 'verify-msg err';
  msg.textContent = ok ? T('security.verify_ok') : T('security.verify_bad');
});

// Show phrase (advanced): asks for the password, decrypts the phrase and shows it once.
$('#sec-reveal').addEventListener('click', () => {
  $('#reveal-pass').value = ''; $('#reveal-msg').hidden = true;
  closeModal('modal-security'); openModal('modal-reveal');
});
$('#reveal-go').addEventListener('click', async () => {
  const pass = $('#reveal-pass').value;
  const msg = $('#reveal-msg'); msg.hidden = false; msg.className = 'verify-msg err';
  if (!pass) { msg.textContent = T('security.reveal_need'); return; }
  const btn = $('#reveal-go'); btn.disabled = true;
  const r = await window.brisvia.wallet.revealSeed(pass);
  btn.disabled = false;
  if (r && Array.isArray(r.words) && r.words.length === 12) {
    const grid = $('#seed-grid-view'); grid.innerHTML = '';
    r.words.forEach((w) => { const li = document.createElement('li'); li.textContent = w; grid.appendChild(li); });
    msg.hidden = true; closeModal('modal-reveal'); openModal('modal-seed');
  } else {
    msg.textContent = r && r.error ? transError(r.error) : T('security.reveal_bad');
  }
});

// ===================== Modals =====================
function openModal(id) { $('#' + id).hidden = false; }
function closeModal(id) { $('#' + id).hidden = true; }
$$('[data-close]').forEach((b) => b.addEventListener('click', (e) => { const ov = e.target.closest('.overlay'); if (ov) ov.hidden = true; }));
// Pop-ups do NOT close on an outside click (they persist, so they aren't dismissed by accident and lose typed input):
// they close only via the X, the buttons (Cancel/etc.) or the Escape key.
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  const open = $$('.overlay').filter((ov) => ov.id !== 'setup' && !ov.hidden).pop();
  if (open) open.hidden = true;
});

// ===================== Network status (real node mode) =====================
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
function setNet(connected, key) {
  const dot = document.querySelector('.net-dot');
  // The "live" class turns on the connected color and the pulse animation (both defined in CSS).
  if (dot) dot.classList.toggle('live', connected);
  const lbl = $('#net-label');
  if (lbl) lbl.textContent = T(key);
}
let syncing = false, syncProgress = 0;
// Whether THIS build targets the real network (mainnet). Read from the node's reported network
// (node_info().network === 'brisvia'), which comes from the compile-time build, so it is right even
// before the node connects. A testnet/preview build keeps this false and is NEVER put in wait mode
// (testnet users must be able to mine right now). `netInfoReceived` avoids flashing the wrong notice
// on a real build before the first node_info arrives.
let isMainnetBuild = false, netInfoReceived = false;
// Human label for the network this build belongs to (test vs real). Solo mode is the only one today
// (you mine against your own node); when pools land, the "mode" row will show the chosen pool.
function networkLabel(net) {
  if (net === 'brisvia') return T('net_panel.net_main');
  return T('net_panel.net_test'); // brisvia-test (and the preview build) → test network
}
async function pollNet() {
  if (!window.brisvia.isReal) return;
  const info = await window.brisvia.nodeInfo();
  const st = await window.brisvia.nodeStatus();
  // Which network this build targets (real vs test). Comes from the build (NET_CHAIN), so it is right
  // even before the node connects. Drives the wait mode + the launch notice below.
  if (info && info.network) { isMainnetBuild = info.network === 'brisvia'; netInfoReceived = true; }
  const connected = !!(info && info.connected);
  const walletReady = !!(st && st.walletReady);
  // Wait mode (real-network build, before launch): the node may still be catching up, but we must NOT show
  // "Syncing" — before the launch date the honest state is "waiting for launch", not a sync in progress.
  const waitMode = isWaitMode();
  // Syncing = connected but still catching up with the shared chain (do not mine yet). Never in wait mode.
  syncing = !waitMode && connected && !!(info && info.ibd);
  syncProgress = (info && info.verificationprogress != null) ? Number(info.verificationprogress) : 0;
  // Green/"connected" only with at least one real connection, so it never shows "connected" next to
  // "peers: 0" (audit N12). Without peers we keep showing "connecting" (still looking for other computers).
  const hasPeers = !!(info && (info.peers ?? 0) > 0);
  const netKey = waitMode ? 'wait.net'
    : (!connected ? 'net.connecting' : (syncing ? 'net.syncing' : (!hasPeers ? 'net.connecting' : (walletReady ? 'net.connected' : 'net.preparing'))));
  setNet(connected && !waitMode && !syncing && hasPeers, netKey);
  if ($('#nr-status')) {
    // Network + mode: gives the user certainty about WHERE they are mining (asked by users who weren't sure).
    // `network` comes from the build itself (NET_CHAIN), so it's right even before the node connects.
    $('#nr-network').textContent = networkLabel(info && info.network);
    // Show what the user picked, honestly (audit N5 + Fernando pto3): mining runs solo until the pool
    // client lands, so pool/custom are shown as "solo · <group> pending".
    $('#nr-mode').textContent = currentMiningMode === 'pool' ? T('net_panel.mode_pool_pending')
      : currentMiningMode === 'custom' ? T('net_panel.mode_custom_pending')
      : T('net_panel.mode_solo');
    $('#nr-status').textContent = waitMode ? T('wait.net')
      : (!connected ? T('net_panel.connecting') : (syncing ? T('net.syncing') : (!hasPeers ? T('net_panel.connecting') : (walletReady ? T('net_panel.connected') : T('net_panel.preparing')))));
    $('#nr-height').textContent = connected ? window.I18N.fmtNum(info.blocks ?? 0) : '—';
    $('#nr-peers').textContent = connected ? (info.peers ?? 0) : '—';
    // Difficulty on the shared testnet is a tiny number; 2 decimals would round it to "0".
    // Show it readable: >=1 with 2 decimals, small values with 3 significant digits.
    $('#nr-diff').textContent = (connected && info.difficulty != null)
      ? (info.difficulty >= 1 ? window.I18N.fmtNum(info.difficulty, { maximumFractionDigits: 2 })
         : info.difficulty === 0 ? '0' : info.difficulty.toLocaleString(window.I18N.lang, { maximumSignificantDigits: 3 }))
      : '—';
  }
  // Mining indicator in the network panel, so it's visible on the Wallet view too (not only on Mine).
  if ($('#nr-mining')) {
    try {
      const ms = await window.brisvia.getStatus();
      const on = !!(ms && ms.mining);
      $('#nr-mining').textContent = on ? T('net_panel.mining_on') : T('net_panel.mining_off');
      $('#nr-mining').className = on ? 'nr-on' : 'nr-off';
    } catch {}
  }
  // Keep the launch notice in sync now that we know which network this build targets.
  updateTestnetBanner();
}

// On language change: re-apply static texts (i18n handles that) + re-render the dynamic ones.
document.addEventListener('langchange', () => {
  renderOnb();
  refreshMine();
  refreshPowLabel();
  applyMiningMode(currentMiningMode); // keep the mining-mode explanation in the current language
  updateTestnetBanner();
  const wv = document.querySelector('.view[data-view="wallet"]');
  if (wv && !wv.hidden) loadWallet();
  // Re-render the achievements (names, descriptions, family labels) in the new language if the view is open.
  const av = document.querySelector('.view[data-view="achievements"]');
  if (av && !av.hidden && achData) renderAchievements(achData.list);
  // Re-translate the balance explanation if it's open (it's set on click, so it wouldn't refresh otherwise).
  const ex = $('#bal-explain');
  if (ex && !ex.hidden && ex.dataset.k) ex.textContent = T(ex.dataset.k);
  $$('#set-language .seg-btn').forEach((b) => b.classList.toggle('active', b.dataset.lang === window.I18N.lang));
});

// ===================== Tooltips (hover explanations) =====================
// Any element with data-tip-key="X" shows the tips.X text ~0.5s after the mouse rests on it.
// Explains in plain words terms a non-technical user may not know (Blocks, Difficulty, etc.).
(function initTooltips() {
  let el = null, timer = null;
  function box() {
    if (!el) { el = document.createElement('div'); el.className = 'tooltip'; document.body.appendChild(el); }
    return el;
  }
  function show(target) {
    const key = target.getAttribute('data-tip-key');
    if (!key) return;
    const txt = T('tips.' + key);
    if (!txt || txt.charAt(0) === '[') return; // no translation available: skip
    const b = box();
    b.textContent = txt;
    b.classList.add('on');
    const r = target.getBoundingClientRect();
    let left = r.left;
    if (left + b.offsetWidth > window.innerWidth - 8) left = window.innerWidth - 8 - b.offsetWidth;
    if (left < 8) left = 8;
    let top = r.bottom + 8;
    if (top + b.offsetHeight > window.innerHeight - 8) top = r.top - b.offsetHeight - 8;
    b.style.left = left + 'px';
    b.style.top = top + 'px';
  }
  function hide() { if (timer) { clearTimeout(timer); timer = null; } if (el) el.classList.remove('on'); }
  document.addEventListener('mouseover', (e) => {
    const t = e.target.closest('[data-tip-key]');
    if (!t) return;
    hide();
    timer = setTimeout(() => show(t), 500);
  });
  document.addEventListener('mouseout', (e) => { if (e.target.closest('[data-tip-key]')) hide(); });
  document.addEventListener('click', hide);
})();

// ===================== Launch gate: wait mode + countdown to real mining =====================
// Real mining start on the REAL network (mainnet). Single source of truth for the client-side "wait mode".
// Kept in sync with the backend MAINNET_START (src-tauri/src/lib.rs). This is only a UX convenience:
// the real protection is the network consensus, not this client.
//   Unix seconds: 1785596400   ·   ISO: 2026-08-01T15:00:00Z   (12:00 Argentina)
const MAINNET_START = Date.UTC(2026, 7, 1, 15, 0, 0); // = 1785596400 * 1000 ms

// Wait mode: a REAL-network build opened BEFORE the launch instant. The wallet works normally; only the
// "Mine" button is held until launch. A testnet/preview build is never in wait mode (people test now).
// Automatic by date: the moment the clock passes MAINNET_START, mining enables itself with no user action.
function isWaitMode() { return isMainnetBuild && Date.now() < MAINNET_START; }

// Human countdown to launch ("Real mining in Xd Yh" / "…about to begin"). Reused by the banner and the Mine view.
function launchCountdownText() {
  const diff = MAINNET_START - Date.now();
  const days = Math.floor(diff / 86400000);
  const hours = Math.floor((diff % 86400000) / 3600000);
  return days >= 1 ? T('testnet.mainnet_in', { d: days, h: hours }) : T('testnet.mainnet_soon');
}

// Top notice shown before launch. On a REAL-network build it is the "wait mode" notice (mining opens Aug 1,
// the wallet works now). On a testnet/preview build it stays the test-network notice. It hides ITSELF the
// moment real mining begins, with no update required.
function updateTestnetBanner() {
  const banner = $('#testnet-banner');
  if (!banner) return;
  const diff = MAINNET_START - Date.now();
  if (diff <= 0) { banner.hidden = true; return; } // launch reached: the notice hides itself
  // On a real build, wait until we know which network this build targets before showing anything, so a
  // mainnet build never flashes the "test network" copy. A preview (no real node) shows the test notice.
  if (window.brisvia.isReal && !netInfoReceived) { banner.hidden = true; return; }
  const tag = $('#tb-tag'), note = $('#tb-note'), until = banner.querySelector('.tb-until'), wallet = banner.querySelector('.tb-wallet');
  if (isMainnetBuild) {
    if (tag) tag.textContent = T('wait.tag');
    if (note) note.textContent = T('wait.note');
    if (until) until.hidden = true; // "the test runs until…" does not apply to the real network
    if (wallet) wallet.textContent = T('wait.wallet_note');
  } else {
    if (tag) tag.textContent = T('testnet.tag');
    if (note) note.textContent = T('testnet.note');
    if (until) { until.hidden = false; until.textContent = T('testnet.until'); }
    if (wallet) wallet.textContent = T('testnet.wallet_note');
  }
  const cd = $('#tb-countdown');
  if (cd) { cd.hidden = false; cd.textContent = launchCountdownText(); }
  banner.hidden = false;
}
setInterval(updateTestnetBanner, 60000); // refresh the countdown every minute

// ===================== Startup =====================
async function init() {
  // Language: the saved one; or the OS one on first run (systemLocale from the real backend; navigator.language in preview).
  let lang = null;
  try { lang = localStorage.getItem('brv_lang'); } catch {}
  if (!lang) {
    let sysLoc = 'en';
    try { sysLoc = (window.brisvia.systemLocale ? await window.brisvia.systemLocale() : navigator.language) || 'en'; } catch {}
    lang = String(sysLoc).toLowerCase().startsWith('es') ? 'es' : 'en';
  }
  window.I18N.setLang(lang);
  if (window.brisvia.setLanguage) window.brisvia.setLanguage(lang);
  updateTestnetBanner();
  // Real version (from app_version), shown in the header chip and in the Settings footer — never a hard-coded number.
  try {
    const v = window.brisvia.appVersion ? await window.brisvia.appVersion() : null;
    if (v) {
      const label = 'v' + v;
      const chip = $('#ver-chip'); if (chip) chip.textContent = label;
      const foot = $('#ver-label'); if (foot) foot.textContent = label;
    }
  } catch {}
  // Auto-check for updates on startup (non-blocking): if there is a newer version, the notice appears.
  setTimeout(() => { checkForUpdate(false); }, 3000);
  // Also check periodically so someone who leaves the app open for days still gets the update pop-up on its own.
  setInterval(() => { checkForUpdate(false); }, 6 * 60 * 60 * 1000); // every 6 hours

  if (window.brisvia.isReal) {
    $('#setup').hidden = true;
    // Decide the first screen WITHOUT waiting for the node: the welcome/onboarding does not need a
    // connected node, so if there is no wallet on disk yet we show it immediately instead of waiting
    // for the node-status loop below (which can take a while before the seed nodes are reachable).
    // Fail CLOSED (ChatGPT, absolute priority): if we cannot tell whether a wallet exists, assume it DOES,
    // so a read glitch never shows the welcome/onboarding on top of an existing wallet. Default true; only a
    // definitive "no seed on disk" (false) opens the first-run flow.
    let walletExists = true;
    try { walletExists = await window.brisvia.wallet.seedOnDisk(); } catch { walletExists = true; }
    if (!walletExists) {
      $('#setup').hidden = false;
      onbStep = 0; renderOnb(); setupStep('welcome');
      setInterval(pollNet, 3000);
    } else {
    setNet(false, 'net.connecting');
    let ready = false, decided = false, walletOnDisk = false;
    for (let i = 0; i < 150 && !decided; i++) {
      await pollNet();
      const s = await window.brisvia.nodeStatus();
      if (s && s.connected) {
        if (s.walletOnDisk) walletOnDisk = true;
        if (s.walletReady) { ready = true; decided = true; }
        else if (s.walletOnDisk === false) { decided = true; }
      }
      if (!decided) await sleep(1000);
    }
    if (ready) {
      $('#setup').hidden = true;
      showView('wallet');
      loadWallet();
      // Offer to protect an OLD wallet (created before password support) with a password.
      try { const k = await window.brisvia.wallet.kind(); if (k && k.encrypted === false) openProtect(); } catch {}
      // Resume mining automatically if the app was mining when it restarted for an update.
      // Never auto-resume during wait mode (mining is not open yet on a real-network build before launch).
      try {
        if (localStorage.getItem('brv_resume_mining')) {
          localStorage.removeItem('brv_resume_mining');
          if (!isWaitMode()) {
            await window.brisvia.start(currentIntensity());
            refreshMine();
          }
        }
      } catch {}
    } else if (walletOnDisk) {
      // Wallet exists on disk but the node hasn't finished loading it yet (slow/syncing node).
      // Do NOT fall back to onboarding: go to the wallet view; pollNet + the refresh interval
      // load it once the node reports it ready. Prevents the welcome tour from reappearing.
      $('#setup').hidden = true;
      showView('wallet');
      loadWallet();
    } else {
      $('#setup').hidden = false;
      onbStep = 0; renderOnb(); setupStep('welcome');
    }
    setInterval(pollNet, 3000);
    }
  } else {
    const exists = await window.brisvia.wallet.exists();
    if (exists && localStorage.getItem('brisvia_onboarded') === '1') {
      $('#setup').hidden = true;
      showView('wallet');
    } else {
      $('#setup').hidden = false;
      onbStep = 0; renderOnb(); setupStep('welcome');
    }
  }
  refreshMine();
  setInterval(refreshMine, 1000);
  setInterval(() => {
    const wv = document.querySelector('.view[data-view="wallet"]');
    if (window.brisvia.isReal && wv && !wv.hidden) loadWallet();
  }, 4000);
  // Achievements: an initial read plus a periodic poll so the unlock toast can appear from any view.
  fetchAchievements();
  setInterval(fetchAchievements, 20000);
}

// Protect an old wallet (no password) — migrates to a password (encryptwallet + encrypted phrase).
function openProtect() {
  $('#protect-1').value = ''; $('#protect-2').value = ''; $('#protect-msg').hidden = true;
  $('#protect-meter').className = 'pass-meter';
  openModal('modal-protect');
}
$('#protect-1').addEventListener('input', () => {
  const p = $('#protect-1').value;
  $('#protect-meter').className = 'pass-meter lvl-' + (p ? passStrength(p) : 0);
});
// Clear a stale error as soon as the user edits either field.
['#protect-1', '#protect-2'].forEach((s) => {
  const el = $(s);
  if (el) el.addEventListener('input', () => { const m = $('#protect-msg'); if (m && !m.hidden) m.hidden = true; });
});
$('#protect-go').addEventListener('click', async () => {
  const p1 = $('#protect-1').value, p2 = $('#protect-2').value;
  const msg = $('#protect-msg'); msg.hidden = false; msg.className = 'verify-msg err';
  if ([...p1].length < 6) { msg.textContent = T('onboarding.pass_weak'); return; } // [...] counts code points, matching backend MIN_PASSWORD_LEN
  if (p1 !== p2) { msg.textContent = T('onboarding.pass_mismatch'); return; }
  const btn = $('#protect-go'); btn.disabled = true;
  const r = await window.brisvia.wallet.migrateEncrypt(p1);
  btn.disabled = false;
  if (r && r.ok) { walletEncrypted = true; msg.hidden = true; closeModal('modal-protect'); loadWallet(); }
  else { msg.textContent = (r && r.error) ? transError(r.error) : T('protect.fail'); }
});

init();
