// Recorrido P0 #12 — Configuración: intensidad de CPU y modo solo/grupo.
// Sobre la app COMPILADA real, verifica que en Ajustes se pueden tocar las opciones y la UI responde:
//   - elegir una intensidad de CPU (p. ej. 75%) marca ese botón como activo;
//   - pasar a modo "grupo" (pool) marca ese botón activo y muestra la fila con los datos del pool;
//   - volver a "solo" oculta esa fila.
// No toca opciones que llaman al sistema operativo (arranque con Windows / bandeja) para no depender del entorno.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Recorrido 12 — configuración', () => {
  it('cambia intensidad de CPU y modo solo/grupo y la UI responde', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);
    await (await $('.nav-btn[data-view="settings"]')).click();
    await (await $('#set-intensity')).waitForDisplayed({ timeout: 10000 });

    // 1) Intensidad de CPU: elegir 75% -> ese botón queda activo.
    const int75 = await $('#set-intensity .seg-btn[data-pct="75"]');
    await int75.click();
    await browser.waitUntil(async () => ((await int75.getAttribute('class')) || '').includes('active'), {
      timeout: 5000, timeoutMsg: 'la intensidad 75% no quedó activa',
    });

    // 2) Modo grupo (pool): el botón queda activo y aparece la fila con los datos del pool.
    const modePool = await $('#set-mining-mode .seg-btn[data-mode="pool"]');
    await modePool.click();
    await browser.waitUntil(async () => ((await modePool.getAttribute('class')) || '').includes('active'), {
      timeout: 5000, timeoutMsg: 'el modo grupo no quedó activo',
    });
    await (await $('#pool-info-row')).waitForDisplayed({ timeout: 5000 });

    // 3) Volver a modo solo: la fila del pool se oculta.
    const modeSolo = await $('#set-mining-mode .seg-btn[data-mode="solo"]');
    await modeSolo.click();
    await browser.waitUntil(async () => !(await (await $('#pool-info-row')).isDisplayed()), {
      timeout: 5000, timeoutMsg: 'la fila del pool no se ocultó al volver a solo',
    });
  });
});
