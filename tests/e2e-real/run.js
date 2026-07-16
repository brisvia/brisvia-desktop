// Runner for Brisvia's REAL E2E testing (layers 3 and 4).
//
// Why it exists: the app binary is launched by tauri-driver, which WebdriverIO starts in its MAIN PROCESS
// (onPrepare hook). That process inherits the environment of whoever invoked it. So, to give each journey
// its own data folder, RPC port, chain (regtest) and test clock, we run wdio ONCE PER SPEC from here,
// setting the environment BEFORE invoking wdio. Between specs we clean everything (node + orphans + temp).
//
// Usage:
//   node tests/e2e-real/run.js                 -> runs the 6 P0 journeys in order
//   node tests/e2e-real/run.js --only 01,05    -> runs only the specs whose name contains 01 or 05
'use strict';

const path = require('path');
const os = require('os');
const { spawnSync } = require('child_process');
const harness = require('./helpers/harness');

const ROOT = harness.ROOT;
const CONFIG = path.join(ROOT, 'wdio.conf.js');
const SPEC_DIR = path.join(ROOT, 'tests', 'e2e-real', 'specs');

// Unix instants (seconds) around the real launch (2026-08-01 15:00 UTC = 1785596400).
const MAINNET_START = 1785596400;
const BEFORE_LAUNCH = MAINNET_START - 3600; // Aug 1 14:00 UTC -> still waiting
const AFTER_LAUNCH = MAINNET_START + 3600;  // Aug 1 16:00 UTC -> already enabled

// Journey plan. app: which binary; regtest: isolated regtest node; nowUnix: frozen clock (wait mode).
const PLAN = [
  { file: '01-first-launch.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '02-create-wallet.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '03a-wait-mode-before.spec.js', app: harness.APP_MAINNET_E2E, regtest: true, nowUnix: BEFORE_LAUNCH },
  { file: '03b-wait-mode-after.spec.js', app: harness.APP_MAINNET_E2E, regtest: true, nowUnix: AFTER_LAUNCH },
  { file: '04-regtest-node.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '05-mining.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '06-close-recovery.spec.js', app: harness.APP_E2E, regtest: true },
  // Wallet journeys (real backend without a node): restore, backup/receive, security, send,
  // language, settings and re-entry.
  { file: '07-restore-wallet.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '08-backup-receive.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '09-wrong-password.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '10-send.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '11-language.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '12-settings.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '13-reentry.spec.js', app: harness.APP_E2E, regtest: true },
];

// Optional filter --only <substr,substr>
function selected() {
  const idx = process.argv.indexOf('--only');
  if (idx === -1) return PLAN;
  const subs = (process.argv[idx + 1] || '').split(',').map((s) => s.trim()).filter(Boolean);
  return PLAN.filter((p) => subs.some((s) => p.file.includes(s)));
}

// Runs a journey: prepares the environment, invokes wdio once, cleans up. Returns true if it passed.
async function runOne(item, attempt) {
  const tag = item.file.replace('.spec.js', '').replace(/[^\w-]+/g, '_');
  const port = await harness.freePort();
  const datadir = harness.makeDatadir(tag);
  const run = { datadir, port, subdir: item.regtest ? 'regtest' : 'main' };

  const env = {
    ...process.env,
    ...harness.envFor({ datadir, port, regtest: item.regtest, nowUnix: item.nowUnix, app: item.app }),
    // Make sure cargo is on PATH (the service may need it for tauri-driver).
    PATH: `${path.join(os.homedir(), '.cargo', 'bin')}${path.delimiter}${process.env.PATH}`,
  };

  const label = attempt > 1 ? `${item.file} (retry ${attempt - 1})` : item.file;
  console.log(`\n=======================================================`);
  console.log(`▶ Journey: ${label}`);
  console.log(`  binary: ${path.basename(item.app)} | regtest: ${item.regtest} | clock: ${item.nowUnix ?? 'real'}`);
  console.log(`  datadir: ${datadir} | rpc: ${port}`);
  console.log(`=======================================================`);

  const npx = process.platform === 'win32' ? 'npx.cmd' : 'npx';
  const r = spawnSync(npx, ['wdio', 'run', CONFIG, '--spec', path.join(SPEC_DIR, item.file)], {
    cwd: ROOT,
    env,
    stdio: 'inherit',
    shell: true,
  });

  const cleanup = await harness.teardown(run);
  if (cleanup && !cleanup.cleanExit) {
    console.log(`  ⚠ NOT a clean exit: ${cleanup.orphans} orphan process(es) had to be forced (node/miner).`);
  } else {
    console.log('  clean exit: 0 orphan processes.');
  }
  return r.status === 0;
}

(async () => {
  const plan = selected();
  if (!plan.length) {
    console.error('No journeys selected.');
    process.exit(2);
  }
  const results = [];
  for (const item of plan) {
    // At most 1 retry: if it only passes on retry, it is still flagged as flaky in the summary.
    let passed = await runOne(item, 1);
    let retried = false;
    if (!passed) {
      retried = true;
      passed = await runOne(item, 2);
    }
    results.push({ file: item.file, passed, retried });
  }

  console.log(`\n\n================  REAL E2E SUMMARY  ================`);
  for (const r of results) {
    const mark = r.passed ? (r.retried ? 'PASSED (on retry = flaky)' : 'PASSED') : 'FAILED';
    console.log(`  ${r.passed ? '✔' : '✖'} ${r.file.padEnd(34)} ${mark}`);
  }
  const failed = results.filter((r) => !r.passed).length;
  console.log(`===================================================`);
  console.log(`  ${results.length - failed}/${results.length} journeys green\n`);
  process.exit(failed ? 1 : 0);
})();
