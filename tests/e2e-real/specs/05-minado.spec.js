// Recorrido P0 #5 — Minado real RandomX (el central).
// Sobre la app COMPILADA real (backend Rust + bitcoind regtest + motor RandomX brisvia-worker), verifica que:
//   - se INICIA el motor de verdad (el mismo comando que dispara el botón "Minar") y el backend reporta minando;
//   - se DETIENE de verdad (el motor para: getStatus().mining == false);
//   - BEST-EFFORT: intenta minar >=1 bloque regtest y, si lo logra, confirma que la altura del nodo sube.
//
// Por qué el bloque es best-effort: RandomX es pesado en el runner gratis de CI (2 hilos, modo liviano),
// y aun a dificultad mínima puede no encontrar un bloque en la ventana de tiempo. El minado real YA está
// validado por otras vías (tests unit del motor, minado de la testnet real, verificador de la pool 15/15
// contra bloques reales). Acá lo central y determinístico es que la app ENCIENDE y APAGA el motor bien
// (la unión JS -> Tauri -> sidecar -> limpieza). El recorrido corre en continue-on-error.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

const isMining = () => browser.executeAsync((done) => {
  window.brisvia.getStatus().then((s) => done(!!(s && s.mining))).catch(() => done(false));
});

describe('Recorrido 5 — minado RandomX', () => {
  it('enciende el motor, intenta minar 1 bloque y lo apaga', async function () {
    this.timeout(360000); // margen amplio: dataset RandomX + intento de minar en un runner CPU-limitado
    const run = harness.fromEnv();

    await harness.onboardCreate(PASSWORD);
    await harness.waitRpcUp(run.datadir, run.port, 60000);
    const h0 = harness.blockCount(run.datadir, run.port);

    await (await $('.nav-btn[data-view="mine"]')).click();
    await (await $('[data-testid="view-mine"]')).waitForDisplayed({ timeout: 10000 });

    // 1) Iniciar el motor (mismo comando que el botón; el botón queda deshabilitado por IBD en regtest génesis).
    const started = await browser.executeAsync((done) => {
      window.brisvia.start('50').then((r) => done(r || true)).catch((e) => done({ error: String(e) }));
    });
    expect(started && !started.error).toBeTruthy();

    // El backend reporta el motor activo (arranque real del sidecar RandomX).
    await browser.waitUntil(async () => await isMining(), {
      timeout: 30000, timeoutMsg: 'el backend no reportó el minado activo tras iniciar',
    });

    // 2) BEST-EFFORT: darle hasta 3 minutos a que mine un bloque (la altura del nodo sube). No falla si no entra.
    let mined = false;
    const t0 = Date.now();
    try {
      await browser.waitUntil(async () => harness.blockCount(run.datadir, run.port) > h0, {
        timeout: 180000, interval: 3000, timeoutMsg: 'no minó en la ventana',
      });
      mined = true;
    } catch { mined = false; }
    const secs = Math.round((Date.now() - t0) / 1000);
    const h1 = harness.blockCount(run.datadir, run.port);
    console.log(`[e2e][05] motor iniciado OK. Bloque minado: ${mined ? 'SÍ' : 'no (runner lento)'} — altura ${h0} -> ${h1} en ~${secs}s.`);
    if (mined) expect(h1).toBeGreaterThan(h0);

    // 3) Detener el motor de verdad: para y el backend deja de reportar minando.
    await browser.executeAsync((done) => {
      window.brisvia.stop().then(() => done(true)).catch(() => done(false));
    });
    await browser.waitUntil(async () => !(await isMining()), {
      timeout: 20000, timeoutMsg: 'el motor no se detuvo tras parar',
    });
  });
});
