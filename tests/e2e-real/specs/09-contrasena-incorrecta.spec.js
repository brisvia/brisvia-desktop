// Recorrido P0 #9 — Contraseña incorrecta al revelar la frase.
// Sobre la app COMPILADA real, verifica que si al pedir la frase (Ajustes -> Seguridad -> Revelar)
// se pone una contraseña EQUIVOCADA, el backend la RECHAZA: muestra un mensaje de error y NO revela
// las 12 palabras (la ventana de la frase no se abre). Es la protección real del respaldo.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';
const WRONG = 'clave-equivocada-9999';

describe('Recorrido 9 — contraseña incorrecta', () => {
  it('rechaza la contraseña equivocada y no revela la frase', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);

    // Ajustes -> Seguridad -> Revelar frase, con la contraseña EQUIVOCADA.
    await (await $('.nav-btn[data-view="settings"]')).click();
    await (await $('#set-security')).click();
    await (await $('#modal-security')).waitForDisplayed({ timeout: 10000 });
    await (await $('#sec-reveal')).click();
    const revealModal = await $('#modal-reveal');
    await revealModal.waitForDisplayed({ timeout: 10000 });
    await (await $('#reveal-pass')).setValue(WRONG);
    await (await $('#reveal-go')).click();

    // Aparece el mensaje de error y la ventana de la frase NO se abre.
    const msg = await $('#reveal-msg');
    await browser.waitUntil(async () => await msg.isDisplayed() && (await msg.getText()).trim().length > 0, {
      timeout: 10000, timeoutMsg: 'no apareció el mensaje de contraseña incorrecta',
    });
    expect(await (await $('#modal-seed')).isDisplayed()).toBe(false);
  });
});
