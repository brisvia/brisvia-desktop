// Recorrido P0 #13 — Reingreso normal (cerrar y reabrir la app).
// Sobre la app COMPILADA real: se crea una billetera, se anota su dirección, se REABRE la app
// (reloadSession relanza el binario con la misma carpeta de datos) y se verifica que la billetera
// sigue estando (no vuelve al alta) y da la MISMA dirección.
//
// EXPERIMENTAL / en estabilización: reabrir depende de que el estado del navegador embebido (WebView2)
// persista entre lanzamientos en el runner. Si el alta reaparece, es señal de que ese estado no se
// conservó en el entorno de CI (no necesariamente un bug de la billetera). Corre con continue-on-error.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Recorrido 13 — reingreso (cerrar y reabrir)', () => {
  it('reabre la app y conserva la misma billetera y dirección', async () => {
    harness.fromEnv();

    const seed = await harness.onboardCreate(PASSWORD);
    const addr1 = await harness.readReceiveAddress();
    expect(addr1.length).toBeGreaterThan(10);

    // Cerrar y reabrir la app (mismo binario, misma carpeta de datos).
    await browser.reloadSession();

    // Tras reabrir: o vuelve la billetera (lo esperado) o reaparece el alta.
    const walletView = await $('[data-testid="view-wallet"]');
    const welcome = await $('[data-testid="onb-welcome"]');
    await browser.waitUntil(async () => (await walletView.isDisplayed()) || (await welcome.isDisplayed()), {
      timeout: 60000, timeoutMsg: 'la app no volvió ni a la billetera ni al alta tras reabrir',
    });

    // Debe haber vuelto directo a la billetera (recordó la billetera existente).
    expect(await welcome.isDisplayed()).toBe(false);
    await walletView.waitForDisplayed({ timeout: 15000 });

    // Y la dirección para recibir es la misma de antes.
    const addr2 = await harness.readReceiveAddress();
    expect(addr2).toBe(addr1);
    // La semilla creada sigue teniendo 12 palabras (sanity del estado previo).
    expect(seed.length).toBe(12);
  });
});
