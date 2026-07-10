// Runner del testing E2E REAL (capas 3 y 4) de Brisvia.
//
// Por qué existe: el binario de la app lo lanza tauri-driver, que WebdriverIO arranca en su PROCESO PRINCIPAL
// (hook onPrepare). Ese proceso hereda el entorno de quien lo invoca. Entonces, para darle a cada recorrido
// su propia carpeta de datos, puerto RPC, cadena (regtest) y reloj de prueba, corremos wdio UNA VEZ POR SPEC
// desde acá, fijando el entorno ANTES de invocar wdio. Entre specs limpiamos todo (nodo + huérfanos + temporales).
//
// Uso:
//   node tests/e2e-real/run.js                 -> corre los 6 recorridos P0 en orden
//   node tests/e2e-real/run.js --only 01,05    -> corre sólo los specs cuyo nombre contenga 01 o 05
'use strict';

const path = require('path');
const os = require('os');
const { spawnSync } = require('child_process');
const harness = require('./helpers/harness');

const ROOT = harness.ROOT;
const CONFIG = path.join(ROOT, 'wdio.conf.js');
const SPEC_DIR = path.join(ROOT, 'tests', 'e2e-real', 'specs');

// Instantes unix (segundos) alrededor del lanzamiento real (1-ago-2026 15:00 UTC = 1785596400).
const MAINNET_START = 1785596400;
const BEFORE_LAUNCH = MAINNET_START - 3600; // 14:00 UTC del 1-ago -> todavía en espera
const AFTER_LAUNCH = MAINNET_START + 3600;  // 16:00 UTC del 1-ago -> ya habilitado

// Plan de recorridos. app: qué binario; regtest: nodo regtest aislado; nowUnix: reloj congelado (modo espera).
const PLAN = [
  { file: '01-apertura.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '02-crear-billetera.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '03a-modo-espera-antes.spec.js', app: harness.APP_MAINNET_E2E, regtest: true, nowUnix: BEFORE_LAUNCH },
  { file: '03b-modo-espera-despues.spec.js', app: harness.APP_MAINNET_E2E, regtest: true, nowUnix: AFTER_LAUNCH },
  { file: '04-nodo-regtest.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '05-minado.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '06-cierre-recuperacion.spec.js', app: harness.APP_E2E, regtest: true },
  // Recorridos de billetera (backend real sin nodo): restaurar, backup/recibir, seguridad, enviar,
  // idioma, configuración y reingreso.
  { file: '07-restaurar.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '08-backup-recibir.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '09-contrasena-incorrecta.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '10-enviar.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '11-idioma.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '12-configuracion.spec.js', app: harness.APP_E2E, regtest: true },
  { file: '13-reingreso.spec.js', app: harness.APP_E2E, regtest: true },
];

// Filtro opcional --only <substr,substr>
function selected() {
  const idx = process.argv.indexOf('--only');
  if (idx === -1) return PLAN;
  const subs = (process.argv[idx + 1] || '').split(',').map((s) => s.trim()).filter(Boolean);
  return PLAN.filter((p) => subs.some((s) => p.file.includes(s)));
}

// Corre un recorrido: prepara entorno, invoca wdio una vez, limpia. Devuelve true si pasó.
async function runOne(item, attempt) {
  const tag = item.file.replace('.spec.js', '').replace(/[^\w-]+/g, '_');
  const port = await harness.freePort();
  const datadir = harness.makeDatadir(tag);
  const run = { datadir, port, subdir: item.regtest ? 'regtest' : 'main' };

  const env = {
    ...process.env,
    ...harness.envFor({ datadir, port, regtest: item.regtest, nowUnix: item.nowUnix, app: item.app }),
    // Aseguramos cargo en el PATH (el service puede necesitarlo para tauri-driver).
    PATH: `${path.join(os.homedir(), '.cargo', 'bin')}${path.delimiter}${process.env.PATH}`,
  };

  const label = attempt > 1 ? `${item.file} (reintento ${attempt - 1})` : item.file;
  console.log(`\n=======================================================`);
  console.log(`▶ Recorrido: ${label}`);
  console.log(`  binario: ${path.basename(item.app)} | regtest: ${item.regtest} | reloj: ${item.nowUnix ?? 'real'}`);
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
    console.log(`  ⚠ cierre NO limpio: hubo ${cleanup.orphans} proceso(s) huérfano(s) que hubo que forzar (nodo/minero).`);
  } else {
    console.log('  cierre limpio: 0 procesos huérfanos.');
  }
  return r.status === 0;
}

(async () => {
  const plan = selected();
  if (!plan.length) {
    console.error('No hay recorridos seleccionados.');
    process.exit(2);
  }
  const results = [];
  for (const item of plan) {
    // Máximo 1 reintento: si sólo pasa al reintentar, igual queda marcado como inestable en el resumen.
    let passed = await runOne(item, 1);
    let retried = false;
    if (!passed) {
      retried = true;
      passed = await runOne(item, 2);
    }
    results.push({ file: item.file, passed, retried });
  }

  console.log(`\n\n================  RESUMEN E2E REAL  ================`);
  for (const r of results) {
    const mark = r.passed ? (r.retried ? 'PASÓ (con reintento = inestable)' : 'PASÓ') : 'FALLÓ';
    console.log(`  ${r.passed ? '✔' : '✖'} ${r.file.padEnd(34)} ${mark}`);
  }
  const failed = results.filter((r) => !r.passed).length;
  console.log(`===================================================`);
  console.log(`  ${results.length - failed}/${results.length} recorridos en verde\n`);
  process.exit(failed ? 1 : 0);
})();
