// Recorrido P0 #5 — Minado real RandomX (el central).
// Sobre la app COMPILADA real (backend Rust + bitcoind regtest + motor RandomX brisvia-worker), verifica que:
//   - se inicia el minado de verdad (el mismo comando que dispara el botón "Minar");
//   - el motor RandomX mina AL MENOS 1 bloque regtest (dificultad mínima -> rápido) y la ALTURA del nodo sube;
//   - se detiene de verdad (el motor para: getStatus().mining == false).
//
// Nota: en regtest recién arrancado el nodo reporta "sincronizando" (ibd), y por eso el BOTÓN de minar
// queda deshabilitado en la UI (protección de UX). Para ejercer el motor igual, arrancamos/paramos con
// el mismo comando del backend que invoca el botón (window.brisvia.start/stop). El chequeo de "minó" es
// objetivo: la altura del nodo sube. Corre en continue-on-error y reporta el tiempo por si el runner es lento.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

const isMining = () => browser.executeAsync((done) => {
  window.brisvia.getStatus().then((s) => done(!!(s && s.mining))).catch(() => done(false));
});

describe('Recorrido 5 — minado RandomX', () => {
  it('inicia el motor, mina al menos 1 bloque regtest y se detiene', async function () {
    this.timeout(300000); // el arranque del dataset RandomX + minar 1 bloque puede tardar en un runner CPU-limitado
    const run = harness.fromEnv();

    await harness.onboardCreate(PASSWORD);
    await harness.waitRpcUp(run.datadir, run.port, 60000);

    const h0 = harness.blockCount(run.datadir, run.port);
    await (await $('.nav-btn[data-view="mine"]')).click();
    await (await $('[data-testid="view-mine"]')).waitForDisplayed({ timeout: 10000 });

    // Iniciar el minado (mismo comando que el botón). Devuelve cuando el backend aceptó el arranque.
    const started = await browser.executeAsync((done) => {
      window.brisvia.start('50').then((r) => done(r || true)).catch((e) => done({ error: String(e) }));
    });
    expect(started && !started.error).toBeTruthy();

    // El motor está minando (preparando el dataset o ya participando).
    await browser.waitUntil(async () => await isMining(), {
      timeout: 30000, timeoutMsg: 'el backend no reportó el minado activo tras iniciar',
    });

    // Esperar a que el motor RandomX mine al menos 1 bloque: la altura del nodo sube.
    const t0 = Date.now();
    await browser.waitUntil(async () => harness.blockCount(run.datadir, run.port) > h0, {
      timeout: 240000, interval: 2000,
      timeoutMsg: 'el motor no minó ningún bloque en 4 minutos (runner lento o error del motor)',
    });
    const secs = Math.round((Date.now() - t0) / 1000);
    const h1 = harness.blockCount(run.datadir, run.port);
    console.log(`[e2e][05] minado: altura ${h0} -> ${h1} en ~${secs}s`);
    expect(h1).toBeGreaterThan(h0);

    // Detener el minado de verdad: el motor para.
    await browser.executeAsync((done) => {
      window.brisvia.stop().then(() => done(true)).catch(() => done(false));
    });
    await browser.waitUntil(async () => !(await isMining()), {
      timeout: 20000, timeoutMsg: 'el motor no se detuvo tras parar',
    });
  });
});
