// WebdriverIO configuration for Brisvia's REAL E2E testing (layers 3 and 4).
//
// Handles the real COMPILED app (real Rust backend) via @wdio/tauri-service:
//   - driverProvider 'external': usa tauri-driver (se instala solo con cargo) y administra el Edge WebDriver.
//   - autoDownloadEdgeDriver: downloads the msedgedriver compatible with this machine's WebView2.
//
// IMPORTANTE: esta config NO fija la carpeta/puerto/cadena de la corrida. Eso lo hace el runner
// (tests/e2e-real/run.js), que corre wdio UNA VEZ POR SPEC con el entorno ya seteado — porque tauri-driver
// (quien lanza la app) se arranca en el proceso principal de wdio y hereda ESE entorno, no el del worker.
// Here we only read the binary and the folder from the environment.
//
// How to run it:  npm run test:e2e:real   (uses the runner)
'use strict';

const path = require('path');
const harness = require('./tests/e2e-real/helpers/harness');

// Binario a manejar: lo elige el runner por variable de entorno (regtest usa el build e2e; el modo espera, el mainnet+e2e).
const APP = process.env.BRISVIA_E2E_APP || harness.APP_E2E;

exports.config = {
  runner: 'local',
  specs: [path.join(__dirname, 'tests', 'e2e-real', 'specs', '*.spec.js')],
  // En SERIE: los recorridos de nodo/minado no deben pisarse (puertos, dataset RandomX, CPU).
  maxInstances: 1,
  // The runner already runs 1 spec per invocation; the retry is controlled by the runner (max 1). Here without internal retries.
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
        driverProvider: 'external', // tauri-driver + Edge WebDriver administrados por el service
        autoInstallTauriDriver: true, // instala tauri-driver con cargo si falta
        autoDownloadEdgeDriver: true, // downloads the msedgedriver that matches the machine's WebView2
        captureBackendLogs: true, // logs del backend Rust en el reporte
        captureFrontendLogs: true, // console.* del frontend en el reporte
        startTimeout: 60000, // la app arranca el nodo en segundo plano; damos margen
      },
    ],
  ],

  logLevel: 'warn',
  bail: 0,
  waitforTimeout: 20000, // espera por defecto de los waitUntil de WebdriverIO
  connectionRetryTimeout: 120000,
  connectionRetryCount: 3,

  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: {
    ui: 'bdd',
    timeout: 180000, // los recorridos con nodo/minado pueden tardar; el corte real lo hacen los waitFor internos
  },

  // Evidencia en cada fallo (pantalla + logs del nodo/minero + procesos + estado RPC). La corrida se lee del entorno.
  afterTest: async function (test, context, { passed }) {
    if (!passed) {
      await harness.captureFailure(browser, harness.fromEnv(), test.title);
    }
  },
};
