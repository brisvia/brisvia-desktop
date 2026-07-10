// Recorrido P0 #6 — Cierre y recuperación.
// Sobre la app COMPILADA real: se crea una billetera, se abre una OPERACIÓN (el modal de enviar con datos
// a medio cargar) y se CIERRA/REABRE la app en ese momento (reloadSession relanza el binario con la misma
// carpeta de datos). Verifica que:
//   - reabrir no corrompe la billetera: al revelar la frase con la contraseña son las MISMAS 12 palabras;
//   - la app reabre limpia y directo a la billetera (no reaparece el alta ni queda el modal a medias).
// La ausencia de procesos huérfanos tras cerrar la valida el runner (teardown -> countProcs, ver run.js).
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Recorrido 6 — cierre y recuperación', () => {
  it('cierra durante una operación y reabre sin corromper la billetera', async () => {
    harness.fromEnv();

    const seed = await harness.onboardCreate(PASSWORD);

    // 1) Abrir una operación a medio hacer: el modal de enviar con datos parciales, sin confirmar.
    await (await $('.nav-btn[data-view="wallet"]')).click();
    await (await $('#act-send')).click();
    const sendModal = await $('#modal-send');
    await sendModal.waitForDisplayed({ timeout: 10000 });
    await (await $('#send-addr')).setValue('brv1qexampleexampleexampleexample00');
    await (await $('#send-amount')).setValue('5');

    // 2) Cerrar y reabrir la app EN MEDIO de la operación.
    await browser.reloadSession();

    // 3) Reabre limpia y directo a la billetera (no reaparece el alta, no queda el modal a medias).
    const walletView = await $('[data-testid="view-wallet"]');
    const welcome = await $('[data-testid="onb-welcome"]');
    await browser.waitUntil(async () => (await walletView.isDisplayed()) || (await welcome.isDisplayed()), {
      timeout: 60000, timeoutMsg: 'la app no volvió tras cerrar durante la operación',
    });
    expect(await welcome.isDisplayed()).toBe(false);
    await walletView.waitForDisplayed({ timeout: 15000 });
    expect(await (await $('#modal-send')).isDisplayed()).toBe(false); // la operación a medias no quedó abierta

    // 4) La billetera NO se corrompió: revelar la frase da las mismas 12 palabras.
    await (await $('.nav-btn[data-view="settings"]')).click();
    await (await $('#set-security')).click();
    await (await $('#modal-security')).waitForDisplayed({ timeout: 10000 });
    await (await $('#sec-reveal')).click();
    await (await $('#modal-reveal')).waitForDisplayed({ timeout: 10000 });
    await (await $('#reveal-pass')).setValue(PASSWORD);
    await (await $('#reveal-go')).click();
    const seedModal = await $('#modal-seed');
    await seedModal.waitForDisplayed({ timeout: 15000 });
    const grid = await $('#seed-grid-view');
    await browser.waitUntil(async () => (await grid.$$('li')).length === 12, {
      timeout: 10000, timeoutMsg: 'la frase revelada tras reabrir no mostró 12 palabras',
    });
    const revealed = [];
    for (const li of await grid.$$('li')) revealed.push((await li.getText()).trim().replace(/^\d+[.)]?\s*/, ''));
    expect(revealed.join(' ')).toBe(seed.join(' '));
  });
});
