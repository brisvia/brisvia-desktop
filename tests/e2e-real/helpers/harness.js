// Scaffolding for the REAL end-to-end tests (layers 3 and 4) of the Brisvia desktop app.
//
// Architecture, and it matters: the app binary is launched by tauri-driver, and tauri-driver is started
// by WebdriverIO's MAIN process (the onPrepare hook), not by the spec's worker. So the environment the
// app sees is the main process's. To give each run its own directory, port and chain, WebdriverIO is
// run ONCE PER SPEC from a runner (run.js) that sets the environment before invoking wdio. The
// environment therefore flows: runner -> wdio (main) -> tauri-driver -> msedgedriver -> app.
//
// This module holds:
//   - How a run's environment is assembled (envFor) and read back inside the wdio process (fromEnv).
//   - Anti-flaky helpers: wait for real CONDITIONS (RPC up, height rising), never for a fixed sleep.
//   - Teardown: stop the node, kill orphans (bitcoind + miner) by their data directory, clear temporaries.
//   - Evidence capture on every failure.
'use strict';

const fs = require('fs');
const os = require('os');
const net = require('net');
const path = require('path');
const { spawnSync } = require('child_process');

const ROOT = path.resolve(__dirname, '..', '..', '..');
const BIN_DIR = path.join(ROOT, 'src-tauri', 'binaries');
const CLI = path.join(BIN_DIR, 'bitcoin-cli.exe');
const TARGET_DIR = path.join(ROOT, 'src-tauri', 'target', 'debug');
const ARTIFACT_DIR = path.join(ROOT, 'test-results', 'e2e-real');

// Test-only binaries, built by build:e2e. These are NEVER published.
const APP_E2E = path.join(TARGET_DIR, 'brisvia-miner-e2e.exe'); // test network (tprv) -> redirected to regtest
const APP_MAINNET_E2E = path.join(TARGET_DIR, 'brisvia-miner-mainnet-e2e.exe'); // real-network build, for the pre-launch wait

// ----- basics -----

// A free TCP port on localhost: asking for port 0 makes the OS pick one. There is a tiny race between
// getting it and using it, which is fine for tests that run one at a time.
function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const p = srv.address().port;
      srv.close(() => resolve(p));
    });
  });
}

// An empty, throwaway data directory for one run.
function makeDatadir(tag) {
  return fs.mkdtempSync(path.join(os.tmpdir(), `brisvia-e2e-${tag}-`));
}

// Builds the environment the app needs for one isolated run.
//   datadir/port   its own directory and RPC port (BRISVIA_DATADIR / BRISVIA_RPC_PORT)
//   regtest        true -> isolated regtest node (BRISVIA_E2E_CHAIN/SUBDIR); false -> the build as it is
//   nowUnix        if set, freezes the app's clock (Date.now) at that unix instant, IN SECONDS
//   app            which binary to launch (BRISVIA_E2E_APP, read by wdio.conf.js)
function envFor({ datadir, port, regtest = true, nowUnix = null, app = APP_E2E }) {
  const env = {
    BRISVIA_DATADIR: datadir,
    BRISVIA_RPC_PORT: String(port),
    BRISVIA_SOLO: '1', // isolated instance: disables single-instance, or an app already open kills this one
    // Updater off: a dead local endpoint makes the check fail fast and leaves the UI unchanged, without
    // reaching the network.
    BRISVIA_UPDATE_ENDPOINT: 'http://127.0.0.1:1/latest.json',
    BRISVIA_E2E_APP: app,
  };
  if (regtest) {
    env.BRISVIA_E2E_CHAIN = 'regtest';
    env.BRISVIA_E2E_SUBDIR = 'regtest';
  }
  if (nowUnix != null) env.BRISVIA_E2E_NOW = String(nowUnix);
  return env;
}

// Reads the current run from the environment. For use INSIDE the wdio process: specs and hooks.
function fromEnv() {
  const datadir = process.env.BRISVIA_DATADIR;
  const port = parseInt(process.env.BRISVIA_RPC_PORT || '0', 10);
  const subdir = process.env.BRISVIA_E2E_SUBDIR || 'regtest';
  return { datadir, port, subdir };
}

// Runs bitcoin-cli against this run's node. Cookie auth, via -datadir + -rpcport.
function rpc(datadir, port, args) {
  const r = spawnSync(CLI, [`-datadir=${datadir}`, `-rpcport=${port}`, ...args], {
    encoding: 'utf8', timeout: 30000, windowsHide: true,
  });
  return { status: r.status, stdout: (r.stdout || '').trim(), stderr: (r.stderr || '').trim() };
}

// Waits for a CONDITION, never for a fixed sleep. fn (sync or async) returns truthy once it holds.
async function waitFor(fn, { timeout = 30000, interval = 300, msg = 'condition' } = {}) {
  const t0 = Date.now();
  let last;
  while (Date.now() - t0 < timeout) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) { last = e; }
    await new Promise((r) => setTimeout(r, interval));
  }
  throw new Error(`Timed out waiting for: ${msg} (last value: ${JSON.stringify(last)})`);
}

// Waits until the node's RPC answers (getblockcount succeeds).
async function waitRpcUp(datadir, port, timeout = 60000) {
  return waitFor(() => rpc(datadir, port, ['getblockcount']).status === 0, {
    timeout, interval: 500, msg: `the node's RPC to be up on :${port}`,
  });
}

// Current chain height, or -1 if the RPC did not answer.
function blockCount(datadir, port) {
  const r = rpc(datadir, port, ['getblockcount']);
  return r.status === 0 ? parseInt(r.stdout, 10) : -1;
}

// ----- teardown and evidence -----

// Kills child processes (bitcoind + miner) whose command line contains this run's data directory.
function killByDatadir(datadir) {
  if (!datadir) return;
  const needle = datadir.replace(/\\/g, '\\\\');
  const ps = [
    '$ErrorActionPreference="SilentlyContinue";',
    `$dd="${needle}";`,
    'Get-CimInstance Win32_Process |',
    ' Where-Object { ($_.Name -eq "bitcoind.exe" -or $_.Name -eq "brisvia-worker.exe") -and $_.CommandLine -like "*$dd*" } |',
    ' ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }',
  ].join(' ');
  spawnSync('powershell', ['-NoProfile', '-NonInteractive', '-Command', ps], {
    encoding: 'utf8', timeout: 20000, windowsHide: true,
  });
}

// Lists live bitcoind/brisvia processes belonging to this run, for the failure evidence.
function listProcs(datadir) {
  const needle = (datadir || '').replace(/\\/g, '\\\\');
  const ps = [
    '$ErrorActionPreference="SilentlyContinue";',
    `$dd="${needle}";`,
    'Get-CimInstance Win32_Process |',
    ' Where-Object { ($_.Name -like "bitcoind*" -or $_.Name -like "brisvia*") } |',
    ' Select-Object ProcessId,Name,@{n="MatchesDatadir";e={$_.CommandLine -like "*$dd*"}} | Format-Table -AutoSize | Out-String',
  ].join(' ');
  const r = spawnSync('powershell', ['-NoProfile', '-NonInteractive', '-Command', ps], {
    encoding: 'utf8', timeout: 20000, windowsHide: true,
  });
  return (r.stdout || '').trim();
}

// Counts this run's live child processes. Zero means a clean shutdown.
function countProcs(datadir) {
  if (!datadir) return 0;
  const needle = datadir.replace(/\\/g, '\\\\');
  const ps = [
    '$ErrorActionPreference="SilentlyContinue";',
    `$dd="${needle}";`,
    '(@(Get-CimInstance Win32_Process |',
    ' Where-Object { ($_.Name -eq "bitcoind.exe" -or $_.Name -eq "brisvia-worker.exe") -and $_.CommandLine -like "*$dd*" })).Count',
  ].join(' ');
  const r = spawnSync('powershell', ['-NoProfile', '-NonInteractive', '-Command', ps], {
    encoding: 'utf8', timeout: 20000, windowsHide: true,
  });
  return parseInt((r.stdout || '0').trim(), 10) || 0;
}

// Stops the node gracefully and kills whatever is left. Then clears the temporaries, retrying because
// Windows holds file locks for a moment. Never throws: teardown must not be what breaks a run.
// Returns { cleanExit, orphans }. cleanExit is true when the child processes (node + miner) reached
// zero ON THEIR OWN, without being forced -- which is the signal that nothing was orphaned. orphans is
// how many had to be forced.
async function teardown(run) {
  if (!run || !run.datadir) return { cleanExit: true, orphans: 0 };
  const { datadir, port } = run;
  try { rpc(datadir, port, ['stop']); } catch {}
  let cleanExit = false;
  for (let i = 0; i < 20; i++) {
    if (countProcs(datadir) === 0) { cleanExit = true; break; }
    await new Promise((r) => setTimeout(r, 500));
  }
  const orphans = cleanExit ? 0 : countProcs(datadir);
  killByDatadir(datadir);
  for (let i = 0; i < 8; i++) {
    try { fs.rmSync(datadir, { recursive: true, force: true }); break; }
    catch { await new Promise((r) => setTimeout(r, 400)); }
  }
  return { cleanExit, orphans };
}

// Saves evidence when a journey fails: a screenshot, the node's debug.log, the miner's events, and
// procesos vivos + estado del RPC, bajo test-results/e2e-real/<tag>-<timestamp>/.
async function captureFailure(browser, run, testName) {
  try {
    if (!run || !run.datadir) return;
    const stamp = new Date().toISOString().replace(/[:.]/g, '-');
    const safe = String(testName || 'fallo').replace(/[^\w.-]+/g, '_').slice(0, 80);
    const dir = path.join(ARTIFACT_DIR, `${stamp}-${safe}`);
    fs.mkdirSync(dir, { recursive: true });
    try { await browser.saveScreenshot(path.join(dir, 'screenshot.png')); } catch {}
    try {
      const dbg = path.join(run.datadir, run.subdir || 'regtest', 'debug.log');
      if (fs.existsSync(dbg)) fs.copyFileSync(dbg, path.join(dir, 'node-debug.log'));
    } catch {}
    try {
      const ev = path.join(run.datadir, 'miner-events.log');
      if (fs.existsSync(ev)) fs.copyFileSync(ev, path.join(dir, 'miner-events.log'));
    } catch {}
    try {
      const info =
        `procesos:\n${listProcs(run.datadir)}\n\n` +
        `getblockchaininfo:\n${rpc(run.datadir, run.port, ['getblockchaininfo']).stdout}\n`;
      fs.writeFileSync(path.join(dir, 'estado.txt'), info);
    } catch {}
    return dir;
  } catch { /* la evidencia nunca debe tumbar la corrida */ }
}

// ----- UI helpers. These run INSIDE the wdio process and use its globals: $, $$, browser, expect -----

// Clicks through the welcome slides until the "create or import" step.
async function skipWelcome() {
  const welcome = await $('[data-testid="onb-welcome"]');
  await welcome.waitForDisplayed({ timeout: 60000 });
  const choose = await $('[data-testid="onb-choose"]');
  const next = await $('[data-testid="onb-next"]');
  for (let i = 0; i < 5 && !(await choose.isDisplayed()); i++) {
    await next.waitForClickable({ timeout: 10000 });
    await next.click();
    await browser.pause(150); // deja re-renderizar la diapositiva; el corte real es el isDisplayed()
  }
  await choose.waitForDisplayed({ timeout: 10000 });
}

// Creates a wallet by walking the whole onboarding (password -> seed -> backup verification) and
// returns the twelve words. Same flow that journey 02 validates, reused rather than duplicated.
async function onboardCreate(password) {
  await skipWelcome();
  await (await $('[data-testid="onb-create"]')).click();

  const pass = await $('[data-testid="onb-pass"]');
  await pass.waitForDisplayed({ timeout: 10000 });
  await (await $('[data-testid="pass-1"]')).setValue(password);
  await (await $('[data-testid="pass-2"]')).setValue(password);
  await (await $('[data-testid="pass-next"]')).click();

  const seedStep = await $('[data-testid="onb-seed"]');
  await seedStep.waitForDisplayed({ timeout: 30000 });
  const seedGrid = await $('[data-testid="seed-grid"]');
  await browser.waitUntil(async () => (await seedGrid.$$('li')).length === 12, {
    timeout: 30000, timeoutMsg: 'the backend did not return twelve words',
  });
  const seed = [];
  for (const li of await seedGrid.$$('li')) seed.push((await li.getText()).trim());
  expect(seed.filter(Boolean).length).toBe(12);

  await (await $('[data-testid="seed-ack"]')).click();
  const seedNext = await $('[data-testid="seed-next"]');
  await browser.waitUntil(async () => await seedNext.isEnabled(), { timeout: 5000 });
  await seedNext.click();

  const verifyStep = await $('[data-testid="onb-verify"]');
  await verifyStep.waitForDisplayed({ timeout: 10000 });
  const slotEls = await $$('[data-testid="verify-slots"] .slot');
  const positions = [];
  for (const s of slotEls) positions.push(parseInt((await s.$('.slot-n').getText()).trim(), 10));
  for (const pos of positions) {
    const word = seed[pos - 1];
    for (const chip of await $$('[data-testid="verify-bank"] .chip')) {
      const cls = (await chip.getAttribute('class')) || '';
      if (cls.includes('used')) continue;
      if ((await chip.getText()).trim() === word) { await chip.click(); break; }
    }
  }

  const setup = await $('#setup');
  await browser.waitUntil(async () => !(await setup.isDisplayed()), {
    timeout: 20000, timeoutMsg: 'onboarding did not close after the backup was verified',
  });
  await (await $('[data-testid="view-wallet"]')).waitForDisplayed({ timeout: 15000 });
  return seed;
}

// Reads the receiving address from an already-open wallet: opens the dialog, waits for the address,
// closes it and returns it. Used to compare addresses across reopens and restores.
async function readReceiveAddress() {
  await (await $('.nav-btn[data-view="wallet"]')).click();
  await (await $('[data-testid="act-receive"]')).click();
  const recvModal = await $('[data-testid="modal-receive"]');
  await recvModal.waitForDisplayed({ timeout: 10000 });
  const addrEl = await $('[data-testid="recv-addr"]');
  await browser.waitUntil(async () => (await addrEl.getText()).trim().length > 10, {
    timeout: 10000, timeoutMsg: 'no receiving address appeared',
  });
  const addr = (await addrEl.getText()).trim();
  await (await recvModal.$('[data-close]')).click();
  return addr;
}

module.exports = {
  ROOT, BIN_DIR, CLI, TARGET_DIR, ARTIFACT_DIR, APP_E2E, APP_MAINNET_E2E,
  freePort, makeDatadir, envFor, fromEnv,
  rpc, waitFor, waitRpcUp, blockCount,
  teardown, killByDatadir, listProcs, countProcs, captureFailure,
  skipWelcome, onboardCreate, readReceiveAddress,
};
