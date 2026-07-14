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
  it('cambia la intensidad de CPU, y el modo pool esta deshabilitado con su motivo a la vista', async () => {
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

    // 2) Minar en pool está APAGADO en la 1.0 (POOL_ENABLED=false en el backend). El motor stratum está
    //    terminado y probado contra la pool real, pero lo que necesita un usuario de pool no existe todavía:
    //    ver la conexión, y la diferencia entre share encontrada, enviada y ACEPTADA. Entregar el motor sin
    //    forma honesta de ver qué hace es cómo alguien termina creyendo que minó horas y no cobró nada.
    //
    //    El botón tiene que estar DESHABILITADO de verdad, no escondido: quien vino buscando pool merece
    //    saber que llega. Y un control que igual reacciona sería peor: prometería algo que no corre.
    const modePool = await $('#set-mining-mode .seg-btn[data-mode="pool"]');
    await browser.waitUntil(async () => !(await modePool.isEnabled()), {
      timeout: 5000,
      timeoutMsg: 'el botón de minar en pool está habilitado: la 1.0 sale sólo con minado individual',
    });

    // 3) El motivo, a la vista. Que esté deshabilitado sin explicar por qué es una pantalla rota.
    await (await $('#pool-soon')).waitForDisplayed({ timeout: 5000 });

    // 4) Clickearlo NO debe hacer nada: ni activarse, ni abrir la fila del pool.
    await modePool.click().catch(() => {}); // un botón deshabilitado puede rechazar el click: da igual
    await browser.pause(300);
    const activo = ((await modePool.getAttribute('class')) || '').includes('active');
    if (activo) throw new Error('el modo pool se activó pese a estar deshabilitado');
    if (await (await $('#pool-info-row')).isDisplayed()) {
      throw new Error('se abrió la fila del pool con el modo pool deshabilitado');
    }

    // 5) Minado individual: sigue siendo elegible, y es lo único que la 1.0 promete.
    const modeSolo = await $('#set-mining-mode .seg-btn[data-mode="solo"]');
    if (!(await modeSolo.isEnabled())) throw new Error('el modo individual quedó deshabilitado');
    await modeSolo.click();
    await browser.waitUntil(async () => ((await modeSolo.getAttribute('class')) || '').includes('active'), {
      timeout: 5000, timeoutMsg: 'el modo individual no quedó activo',
    });
  });
});
