// Andamiaje de los tests E2E REALES (capas 3 y 4) del programa de escritorio Brisvia.
//
// Arquitectura (importante): el binario de la app lo lanza tauri-driver, y tauri-driver lo arranca el
// PROCESO PRINCIPAL de WebdriverIO (hook onPrepare), NO el worker del spec. Por eso el entorno que ve
// la app es el del proceso principal. Para poder darle a cada recorrido su propia carpeta/puerto/cadena,
// corremos WebdriverIO UNA VEZ POR SPEC desde un runner (run.js) que fija el entorno antes de invocar wdio.
// Así, el env fluye: runner -> wdio (principal) -> tauri-driver -> msedgedriver -> app.
//
// Este módulo reúne:
//   - Cómo se arma el entorno de una corrida (envFor) y cómo se lee dentro del proceso wdio (fromEnv).
//   - Helpers "anti-flaky": esperar CONDICIONES reales (RPC arriba, altura que sube), nunca sleeps fijos.
//   - Cierre y limpieza: detener el nodo, matar huérfanos (bitcoind + minero) por su carpeta de datos, borrar temporales.
//   - Captura de evidencia en cada fallo.
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

// Binarios de prueba (compilados por build:e2e). NUNCA se publican.
const APP_E2E = path.join(TARGET_DIR, 'brisvia-miner-e2e.exe'); // red de prueba (tprv) -> se redirige a regtest
const APP_MAINNET_E2E = path.join(TARGET_DIR, 'brisvia-miner-mainnet-e2e.exe'); // build de red real (para modo espera)

// ----- utilidades base -----

// Un puerto TCP libre en localhost (el SO asigna uno en el puerto 0). Ventana de carrera mínima; alcanza para pruebas en serie.
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

// Carpeta de datos temporal y vacía para una corrida.
function makeDatadir(tag) {
  return fs.mkdtempSync(path.join(os.tmpdir(), `brisvia-e2e-${tag}-`));
}

// Arma el objeto de entorno que la app necesita para una corrida aislada.
//   datadir/port   carpeta y puerto RPC propios (BRISVIA_DATADIR / BRISVIA_RPC_PORT)
//   regtest        true  -> nodo regtest aislado (BRISVIA_E2E_CHAIN/SUBDIR); false -> build tal cual
//   nowUnix        si se define, congela el reloj de la app (Date.now) a ese instante unix EN SEGUNDOS
//   app            binario a lanzar (BRISVIA_E2E_APP, lo lee wdio.conf.js)
function envFor({ datadir, port, regtest = true, nowUnix = null, app = APP_E2E }) {
  const env = {
    BRISVIA_DATADIR: datadir,
    BRISVIA_RPC_PORT: String(port),
    BRISVIA_SOLO: '1', // instancia aislada: sin "instancia única" (si no, una app previa mata a la nueva)
    // Updater apagado: endpoint local muerto -> el chequeo falla rápido y la UI sigue igual (sin salir a la red).
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

// Lee la corrida en curso desde el entorno (para usar DENTRO del proceso wdio: specs y hooks).
function fromEnv() {
  const datadir = process.env.BRISVIA_DATADIR;
  const port = parseInt(process.env.BRISVIA_RPC_PORT || '0', 10);
  const subdir = process.env.BRISVIA_E2E_SUBDIR || 'regtest';
  return { datadir, port, subdir };
}

// Ejecuta bitcoin-cli contra el nodo de la corrida (auth por cookie via -datadir + -rpcport).
function rpc(datadir, port, args) {
  const r = spawnSync(CLI, [`-datadir=${datadir}`, `-rpcport=${port}`, ...args], {
    encoding: 'utf8', timeout: 30000, windowsHide: true,
  });
  return { status: r.status, stdout: (r.stdout || '').trim(), stderr: (r.stderr || '').trim() };
}

// Espera genérica por una CONDICIÓN (nunca sleeps fijos). fn (sync o async) debe devolver truthy al cumplirse.
async function waitFor(fn, { timeout = 30000, interval = 300, msg = 'condición' } = {}) {
  const t0 = Date.now();
  let last;
  while (Date.now() - t0 < timeout) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) { last = e; }
    await new Promise((r) => setTimeout(r, interval));
  }
  throw new Error(`Timeout esperando: ${msg} (último valor: ${JSON.stringify(last)})`);
}

// Espera activa a que el RPC del nodo responda (getblockcount OK).
async function waitRpcUp(datadir, port, timeout = 60000) {
  return waitFor(() => rpc(datadir, port, ['getblockcount']).status === 0, {
    timeout, interval: 500, msg: `RPC del nodo arriba en :${port}`,
  });
}

// Altura actual de la cadena (o -1 si el RPC no respondió).
function blockCount(datadir, port) {
  const r = rpc(datadir, port, ['getblockcount']);
  return r.status === 0 ? parseInt(r.stdout, 10) : -1;
}

// ----- limpieza y evidencia -----

// Mata procesos hijos (bitcoind + minero) cuya línea de comando contenga la carpeta de datos de la corrida.
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

// Lista procesos bitcoind/brisvia vivos asociados a la corrida (para la evidencia de fallo).
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

// Cuenta procesos hijos vivos de la corrida (0 = cierre limpio).
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

// Detiene el nodo con gracia y, si algo queda, lo mata. Después borra los temporales (con reintentos por
// los bloqueos de archivo de Windows). No lanza: la limpieza nunca debe romper la corrida.
// Devuelve { cleanExit, orphans }: cleanExit=true si los procesos hijos (nodo+minero) quedaron en 0 SOLOS,
// sin necesidad de forzar (señal de "no dejó procesos huérfanos"). orphans = cuántos hubo que forzar.
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

// Guarda evidencia cuando un recorrido falla: pantalla + debug.log del nodo + eventos del minero +
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

// ----- helpers de UI (se ejecutan DENTRO del proceso wdio: usan los globals $, $$, browser, expect) -----

// Pasa las diapositivas de bienvenida hasta llegar al paso "crear o importar".
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

// Crea una billetera nueva recorriendo el alta completa (contraseña -> semilla -> verificación de
// respaldo) y devuelve las 12 palabras generadas. Reusa el mismo flujo que valida el recorrido 02.
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
    timeout: 30000, timeoutMsg: 'el backend no devolvió 12 palabras',
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
    timeout: 20000, timeoutMsg: 'el alta no se cerró tras verificar el respaldo',
  });
  await (await $('[data-testid="view-wallet"]')).waitForDisplayed({ timeout: 15000 });
  return seed;
}

// Lee la dirección para recibir desde la billetera ya abierta (abre el modal, espera la dirección,
// lo cierra y la devuelve). Sirve para comparar direcciones entre reaperturas/restauraciones.
async function readReceiveAddress() {
  await (await $('.nav-btn[data-view="wallet"]')).click();
  await (await $('[data-testid="act-receive"]')).click();
  const recvModal = await $('[data-testid="modal-receive"]');
  await recvModal.waitForDisplayed({ timeout: 10000 });
  const addrEl = await $('[data-testid="recv-addr"]');
  await browser.waitUntil(async () => (await addrEl.getText()).trim().length > 10, {
    timeout: 10000, timeoutMsg: 'no apareció una dirección para recibir',
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
