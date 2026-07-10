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

  // Antes de cada comando de foco ($, findElement, click, getTitle), @wdio/tauri-service corre
  // ensureActiveWindowFocus(), que consulta el estado de ventanas vía el plugin "wdio" de Tauri.
  // Esta app NO registra ese plugin (no hace falta para manejar una app de UNA sola ventana), así que
  // cada consulta se cuelga ~8s en un reintento de 100 pasos ANTES de fallar, y no cachea el fallo:
  // paga esos ~8s en TODOS los comandos. En un recorrido con muchas interacciones (crear billetera)
  // esa suma revienta el presupuesto de 180s (el recorrido 01, con pocos comandos, sí llegaba).
  // Neutralizamos la consulta: que devuelva [] al instante ("no hay ventanas -> no cambiar foco"),
  // sin alterar lo que los recorridos ejercen (una sola ventana no necesita este manejo de foco).
  before: async function () {
    try {
      const t = browser.tauri;
      if (t && typeof t.execute === 'function' && !t.__brisviaFastFocus) {
        t.execute = async () => [];
        t.__brisviaFastFocus = true;
        console.log('[e2e] foco de ventana del tauri-service neutralizado (app de una sola ventana).');
      }
    } catch (e) {
      // Si el service cambia su API interna, no rompemos la corrida por esto.
      console.log('[e2e] no se pudo neutralizar el foco del tauri-service:', e && e.message);
    }
  },

  // Evidencia en cada fallo (pantalla + logs del nodo/minero + procesos + estado RPC). La corrida se lee del entorno.
  afterTest: async function (test, context, { passed }) {
    if (!passed) {
      await harness.captureFailure(browser, harness.fromEnv(), test.title);
    }
  },
};
