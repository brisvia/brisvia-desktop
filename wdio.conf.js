// Configuración de WebdriverIO para el testing E2E REAL (capas 3 y 4) de Brisvia.
//
// Maneja la app COMPILADA de verdad (backend Rust real) vía @wdio/tauri-service:
//   - driverProvider 'external': usa tauri-driver (se instala solo con cargo) y administra el Edge WebDriver.
//   - autoDownloadEdgeDriver: baja el msedgedriver compatible con el WebView2 de esta máquina.
//
// IMPORTANTE: esta config NO fija la carpeta/puerto/cadena de la corrida. Eso lo hace el runner
// (tests/e2e-real/run.js), que corre wdio UNA VEZ POR SPEC con el entorno ya seteado — porque tauri-driver
// (quien lanza la app) se arranca en el proceso principal de wdio y hereda ESE entorno, no el del worker.
// Acá sólo leemos el binario y la carpeta desde el entorno.
//
// Cómo correrlo:  npm run test:e2e:real   (usa el runner)
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
  // El runner ya corre 1 spec por invocación; el reintento lo controla el runner (máx 1). Acá sin reintentos internos.
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
        autoDownloadEdgeDriver: true, // baja el msedgedriver que matchea el WebView2 de la máquina
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

  // Nota: el manejo de foco por-comando del @wdio/tauri-service (ensureActiveWindowFocus, que corre
  // antes de $/findElement/click/getTitle) se DESACTIVA vía patch-package
  // (patches/@wdio+tauri-service+1.2.0.patch, reaplicado por el script postinstall). Esta app es de
  // UNA sola ventana, así que ese hook es innecesario; y como no registra el plugin "wdio" de Tauri,
  // cada consulta gastaba ~8s antes de fallar, reventando el presupuesto de 180s en los recorridos
  // con muchas interacciones (el recorrido 01, con pocos comandos, sí llegaba; el 02 no).

  // Evidencia en cada fallo (pantalla + logs del nodo/minero + procesos + estado RPC). La corrida se lee del entorno.
  afterTest: async function (test, context, { passed }) {
    if (!passed) {
      await harness.captureFailure(browser, harness.fromEnv(), test.title);
    }
  },
};
