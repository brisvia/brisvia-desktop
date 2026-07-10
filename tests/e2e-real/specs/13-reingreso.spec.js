// Recorrido P0 #13 — Reingreso normal (cerrar y reabrir la app).
// Sobre la app COMPILADA real: se crea una billetera, se REABRE la app (reloadSession relanza el
// binario con la MISMA carpeta de datos) y se verifica que:
//   - la app vuelve DIRECTO a la billetera (no reaparece el alta ni pide crear/importar de nuevo);
//   - es la MISMA billetera: al revelar la frase con la contraseña, son las MISMAS 12 palabras.
//
// Nota importante (no es un bug): la dirección para recibir que muestra la app CAMBIA en cada arranque
// (el backend pide una dirección nueva sin usar al abrir: buena práctica de privacidad; las direcciones
// anteriores siguen siendo propias de la misma billetera). Por eso el chequeo estable de "misma
// billetera" es la FRASE de 12 palabras (determinística), no la dirección mostrada.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Recorrido 13 — reingreso (cerrar y reabrir)', () => {
  it('reabre la app directo a la billetera y conserva la misma frase de 12 palabras', async () => {
    harness.fromEnv();

    const seed = await harness.onboardCreate(PASSWORD);
    expect(seed.length).toBe(12);

    // Cerrar y reabrir la app (mismo binario, misma carpeta de datos).
    await browser.reloadSession();

    // Tras reabrir: debe volver DIRECTO a la billetera, sin reaparecer el alta.
    const walletView = await $('[data-testid="view-wallet"]');
    const welcome = await $('[data-testid="onb-welcome"]');
    await browser.waitUntil(async () => (await walletView.isDisplayed()) || (await welcome.isDisplayed()), {
      timeout: 60000, timeoutMsg: 'la app no volvió ni a la billetera ni al alta tras reabrir',
    });
    expect(await welcome.isDisplayed()).toBe(false); // recordó la billetera existente
    await walletView.waitForDisplayed({ timeout: 15000 });

    // Es la MISMA billetera: revelar la frase con la contraseña devuelve las mismas 12 palabras.
    await (await $('.nav-btn[data-view="settings"]')).click();
    await (await $('#set-security')).click();
    await (await $('#modal-security')).waitForDisplayed({ timeout: 10000 });
    await (await $('#sec-reveal')).click();
    const revealModal = await $('#modal-reveal');
    await revealModal.waitForDisplayed({ timeout: 10000 });
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
