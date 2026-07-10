// Recorrido P0 #8 — Backup (ver de nuevo las 12 palabras) + Recibir (dirección).
// Sobre la app COMPILADA real (backend Rust real, sin depender del nodo), verifica que:
//   - se crea una billetera (backend genera 12 palabras reales) y se entra;
//   - desde Ajustes -> Seguridad se pueden REVELAR las 12 palabras pidiendo la contraseña,
//     y son EXACTAMENTE las mismas que se generaron al crearla (respaldo real, no cosmético);
//   - Recibir muestra una dirección real de la billetera.
// Es "backend real sin nodo": crear/revelar/dirección no dependen de que el nodo esté arriba.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Recorrido 8 — backup y recibir', () => {
  it('revela las mismas 12 palabras con la contraseña y muestra una dirección para recibir', async () => {
    harness.fromEnv();

    const seed = await harness.onboardCreate(PASSWORD);

    // 1) Ir a Ajustes -> abrir Seguridad -> Revelar frase.
    await (await $('.nav-btn[data-view="settings"]')).click();
    const openSecurity = await $('#set-security');
    await openSecurity.waitForClickable({ timeout: 10000 });
    await openSecurity.click();
    await (await $('#modal-security')).waitForDisplayed({ timeout: 10000 });
    await (await $('#sec-reveal')).click();

    // 2) Pedir la frase con la contraseña correcta.
    const revealModal = await $('#modal-reveal');
    await revealModal.waitForDisplayed({ timeout: 10000 });
    await (await $('#reveal-pass')).setValue(PASSWORD);
    await (await $('#reveal-go')).click();

    // 3) Se muestran las 12 palabras y coinciden EXACTAMENTE con las creadas.
    const seedModal = await $('#modal-seed');
    await seedModal.waitForDisplayed({ timeout: 15000 });
    const grid = await $('#seed-grid-view');
    await browser.waitUntil(async () => (await grid.$$('li')).length === 12, {
      timeout: 10000, timeoutMsg: 'la frase revelada no mostró 12 palabras (¿contraseña rechazada?)',
    });
    const revealed = [];
    for (const li of await grid.$$('li')) revealed.push((await li.getText()).trim().replace(/^\d+[.)]?\s*/, ''));
    expect(revealed.join(' ')).toBe(seed.join(' '));

    // Cerrar el modal de la frase.
    await (await seedModal.$('[data-close]')).click();

    // 4) Recibir: se muestra una dirección real de la billetera.
    const addr = await harness.readReceiveAddress();
    expect(addr.length).toBeGreaterThan(10);
  });
});
