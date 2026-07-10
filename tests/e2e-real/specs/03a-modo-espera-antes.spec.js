// Recorrido P0 #3a — Modo espera ANTES del lanzamiento (build de red real + reloj congelado antes del 1-ago-2026).
// Sobre la app COMPILADA real de RED REAL (binario mainnet-e2e) con el reloj fijado ANTES de MAINNET_START,
// verifica que:
//   - la billetera funciona (se puede crear/entrar);
//   - el minado está EN ESPERA: el estado dice "Próximamente" (wait.badge) y el botón de minar está deshabilitado;
//   - el panel de red NO dice "Sincronizando", sino "En espera de lanzamiento" (fix del pendiente conocido).
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Recorrido 3a — modo espera (antes del lanzamiento)', () => {
  it('muestra "en espera", el botón de minar deshabilitado y el panel de red sin "Sincronizando"', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);

    // Textos esperados según el idioma real de la app (locale-proof).
    const waitBadge = await browser.execute(() => window.I18N.t('wait.badge'));
    const waitNet = await browser.execute(() => window.I18N.t('wait.net'));

    // Vista Minar: estado "en espera" + botón deshabilitado (le damos tiempo a pollNet a detectar la red real).
    await (await $('.nav-btn[data-view="mine"]')).click();
    const badge = await $('[data-testid="state-badge"]');
    await browser.waitUntil(async () => (await badge.getText()).trim() === waitBadge, {
      timeout: 20000, timeoutMsg: `el estado no quedó en espera ("${waitBadge}")`,
    });
    expect(await (await $('[data-testid="mine-toggle"]')).isEnabled()).toBe(false);

    // Panel de red (vista billetera): en modo espera dice "En espera de lanzamiento", nunca "Sincronizando".
    await (await $('.nav-btn[data-view="wallet"]')).click();
    const status = await $('[data-testid="nr-status"]');
    await browser.waitUntil(async () => (await status.getText()).trim() === waitNet, {
      timeout: 20000, timeoutMsg: `el panel de red no mostró "${waitNet}" en modo espera`,
    });
  });
});
