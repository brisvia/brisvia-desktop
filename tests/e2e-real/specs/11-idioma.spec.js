// Recorrido P0 #11 — Cambiar idioma (es/en) desde Ajustes.
// Sobre la app COMPILADA real, verifica que al elegir inglés/español en Ajustes la interfaz cambia
// de idioma de verdad: el botón de idioma elegido queda activo y un texto conocido de la UI
// (la pestaña "Billetera"/"Wallet") cambia según el idioma. No asume el idioma inicial.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Recorrido 11 — idioma', () => {
  it('cambia entre inglés y español y la interfaz responde', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);

    const navWallet = await $('[data-testid="nav-wallet"]');
    await (await $('.nav-btn[data-view="settings"]')).click();
    const langSeg = await $('#set-language');
    await langSeg.waitForDisplayed({ timeout: 10000 });

    // Elegir inglés -> el botón EN queda activo y la pestaña dice "Wallet".
    await (await $('#set-language .seg-btn[data-lang="en"]')).click();
    await browser.waitUntil(async () => (await navWallet.getText()).trim() === 'Wallet', {
      timeout: 8000, timeoutMsg: 'la interfaz no pasó a inglés',
    });
    expect((await $('#set-language .seg-btn[data-lang="en"]'))).toBeTruthy();
    const enActive = await (await $('#set-language .seg-btn[data-lang="en"]')).getAttribute('class');
    expect(enActive.includes('active')).toBe(true);

    // Elegir español -> el botón ES queda activo y la pestaña dice "Billetera".
    await (await $('#set-language .seg-btn[data-lang="es"]')).click();
    await browser.waitUntil(async () => (await navWallet.getText()).trim() === 'Billetera', {
      timeout: 8000, timeoutMsg: 'la interfaz no volvió a español',
    });
    const esActive = await (await $('#set-language .seg-btn[data-lang="es"]')).getAttribute('class');
    expect(esActive.includes('active')).toBe(true);
  });
});
