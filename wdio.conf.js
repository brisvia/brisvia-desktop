// WebdriverIO configuration for Brisvia's REAL E2E testing (layers 3 and 4).
//
// Drives the actually COMPILED app (real Rust backend) via @wdio/tauri-service:
//   - driverProvider 'external': uses tauri-driver (installed on its own by cargo) and manages Edge WebDriver.
//   - autoDownloadEdgeDriver: downloads the msedgedriver compatible with this machine's WebView2.
//
// IMPORTANT: this config does NOT set the run's folder/port/chain. That is done by the runner
// (tests/e2e-real/run.js), which runs wdio ONCE PER SPEC with the environment already set -- because
// tauri-driver (which launches the app) starts in wdio's main process and inherits THAT environment, not
// the worker's. Here we only read the binary and the folder from the environment.
//
// How to run it:  npm run test:e2e:real   (uses the runner)
'use strict';

const path = require('path');
const harness = require('./tests/e2e-real/helpers/harness');

// Binary to drive: the runner picks it via an environment variable (regtest uses the e2e build; wait mode, mainnet+e2e).
const APP = process.env.BRISVIA_E2E_APP || harness.APP_E2E;

exports.config = {
  runner: 'local',
  specs: [path.join(__dirname, 'tests', 'e2e-real', 'specs', '*.spec.js')],
  // SERIALLY: the node/mining journeys must not step on each other (ports, RandomX dataset, CPU).
  maxInstances: 1,
  // The runner already runs 1 spec per invocation; the retry is controlled by the runner (max 1). No internal retries here.
  specFileRetries: 0,

  capabilities: [
    {
      browserName: 'tauri',
      'tauri:options': { application: APP },
    },
  ],

  services: [
    [
      '@wdio/tauri-service',
      {
        appBinaryPath: APP,
        driverProvider: 'external', // tauri-driver + Edge WebDriver managed by the service
        autoInstallTauriDriver: true, // installs tauri-driver with cargo if missing
        autoDownloadEdgeDriver: true, // downloads the msedgedriver matching the machine's WebView2
        captureBackendLogs: true, // Rust backend logs in the report
        captureFrontendLogs: true, // frontend console.* in the report
        startTimeout: 60000, // the app starts the node in the background; give it room
      },
    ],
  ],

  logLevel: 'warn',
  bail: 0,
  waitforTimeout: 20000, // default wait for WebdriverIO's waitUntil
  connectionRetryTimeout: 120000,
  connectionRetryCount: 3,

  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: {
    ui: 'bdd',
    timeout: 180000, // node/mining journeys can take a while; the real cutoff is done by the internal waitFor calls
  },

  // Note: the per-command focus handling of @wdio/tauri-service (ensureActiveWindowFocus, which runs
  // before $/findElement/click/getTitle) is DISABLED via patch-package
  // (patches/@wdio+tauri-service+1.2.0.patch, reapplied by the postinstall script). This app is
  // single-window, so that hook is unnecessary; and since it does not register Tauri's "wdio" plugin,
  // each query spent ~8s before failing, blowing the 180s budget on journeys with many interactions
  // (journey 01, with few commands, did make it; 02 did not).

  // Evidence on every failure (screen + node/miner logs + processes + RPC state). The run is read from the environment.
  afterTest: async function (test, context, { passed }) {
    if (!passed) {
      await harness.captureFailure(browser, harness.fromEnv(), test.title);
    }
  },
};
