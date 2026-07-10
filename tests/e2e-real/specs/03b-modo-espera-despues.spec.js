// Recorrido P0 #3b — Modo espera DESPUÉS del lanzamiento (build de red real + reloj congelado después del 1-ago-2026).
// Sobre la app COMPILADA real de RED REAL (binario mainnet-e2e) con el reloj fijado DESPUÉS de MAINNET_START,
// verifica que la app YA NO está en espera: el estado deja de decir "Próximamente" (el cruce de la fecha
// habilita el minado por sí solo, sin acción del usuario). El botón puede quedar deshabilitado un momento por
// la sincronización real del nodo, pero el estado NO debe ser "en espera".
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Recorrido 3b — modo espera (después del lanzamiento)', () => {
  it('ya no está en espera: el estado deja de ser "Próximamente" al pasar la fecha', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);

    // En minúsculas: el badge se muestra en MAYÚSCULAS por CSS y getText() respeta ese estilo.
    const waitBadge = (await browser.execute(() => window.I18N.t('wait.badge'))).toLowerCase();
    const waitTitle = (await browser.execute(() => window.I18N.t('wait.title'))).toLowerCase();

    await (await $('.nav-btn[data-view="mine"]')).click();
    const badge = await $('[data-testid="state-badge"]');
    const hero = await $('[data-testid="hero-title"]');

    // Damos margen a pollNet para detectar la red real y recalcular el modo. El estado NO debe quedar "en espera".
    await browser.waitUntil(async () => {
      const b = (await badge.getText()).trim().toLowerCase();
      const h = (await hero.getText()).trim().toLowerCase();
      return b.length > 0 && b !== waitBadge && h !== waitTitle;
    }, { timeout: 30000, timeoutMsg: 'tras pasar la fecha, la app siguió mostrando el estado "en espera"' });

    expect((await badge.getText()).trim().toLowerCase()).not.toBe(waitBadge);
  });
});
