// Recorrido P0 #2 — Crear billetera (flujo real de alta).
// Sobre la app COMPILADA real (backend Rust genera las 12 palabras y cifra la billetera), verifica que:
//   - el alta (onboarding) recorre bienvenida -> elegir -> contraseña -> semilla -> verificación;
//   - el backend genera 12 palabras reales y las muestra;
//   - la verificación de respaldo (elegir las palabras pedidas en orden) confirma el respaldo real;
//   - al terminar, la app sale del alta y muestra la billetera con la versión cargada desde el backend.
// Es "backend real sin nodo": crear la billetera no depende de que el nodo esté arriba.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234'; // >= 8 caracteres, igual en las dos casillas

describe('Recorrido 2 — crear billetera', () => {
  it('genera 12 palabras reales, verifica el respaldo y entra a la billetera', async () => {
    harness.fromEnv();

    // 1) Bienvenida: la app arranca sin billetera y muestra el alta.
    const welcome = await $('[data-testid="onb-welcome"]');
    await welcome.waitForDisplayed({ timeout: 60000 });

    // 2) Pasar las 3 diapositivas de bienvenida hasta llegar a "crear o importar".
    const choose = await $('[data-testid="onb-choose"]');
    const next = await $('[data-testid="onb-next"]');
    for (let i = 0; i < 5 && !(await choose.isDisplayed()); i++) {
      await next.waitForClickable({ timeout: 10000 });
      await next.click();
      await browser.pause(150); // deja re-renderizar la diapositiva; el corte real es el isDisplayed()
    }
    await choose.waitForDisplayed({ timeout: 10000 });

    // 3) Elegir "crear billetera" -> paso de contraseña.
    const create = await $('[data-testid="onb-create"]');
    await create.waitForClickable({ timeout: 10000 });
    await create.click();

    const pass = await $('[data-testid="onb-pass"]');
    await pass.waitForDisplayed({ timeout: 10000 });
    const p1 = await $('[data-testid="pass-1"]');
    const p2 = await $('[data-testid="pass-2"]');
    await p1.setValue(PASSWORD);
    await p2.setValue(PASSWORD);
    const passNext = await $('[data-testid="pass-next"]');
    await passNext.click();

    // 4) El backend generó las 12 palabras y las muestra. Las leemos para poder verificar el respaldo.
    const seedStep = await $('[data-testid="onb-seed"]');
    await seedStep.waitForDisplayed({ timeout: 30000 });
    const seedGrid = await $('[data-testid="seed-grid"]');
    await browser.waitUntil(async () => (await seedGrid.$$('li')).length === 12, {
      timeout: 30000,
      timeoutMsg: 'el backend no devolvió 12 palabras (¿falló wallet.create?)',
    });
    const seedItems = await seedGrid.$$('li');
    const seed = [];
    for (const li of seedItems) seed.push((await li.getText()).trim());
    expect(seed.filter(Boolean).length).toBe(12);

    // 5) Confirmar que las anotó y avanzar a la verificación.
    const ack = await $('[data-testid="seed-ack"]');
    await ack.click();
    const seedNext = await $('[data-testid="seed-next"]');
    await browser.waitUntil(async () => await seedNext.isEnabled(), {
      timeout: 5000, timeoutMsg: 'el botón de continuar de la semilla no se habilitó tras marcar el check',
    });
    await seedNext.click();

    // 6) Verificación de respaldo: la app pide 3 palabras por su posición. Leemos las posiciones pedidas
    //    y elegimos del banco la palabra correcta, en el orden pedido (posiciones ascendentes).
    const verifyStep = await $('[data-testid="onb-verify"]');
    await verifyStep.waitForDisplayed({ timeout: 10000 });
    const slotEls = await $$('[data-testid="verify-slots"] .slot');
    const positions = [];
    for (const s of slotEls) {
      const n = parseInt((await s.$('.slot-n').getText()).trim(), 10);
      positions.push(n); // 1-indexado, tal como lo muestra la UI
    }
    expect(positions.length).toBe(3);

    for (const pos of positions) {
      const word = seed[pos - 1];
      // Elegir el chip del banco con esa palabra que todavía no fue usado.
      const chips = await $$('[data-testid="verify-bank"] .chip');
      let clicked = false;
      for (const chip of chips) {
        const cls = (await chip.getAttribute('class')) || '';
        if (cls.includes('used')) continue;
        if ((await chip.getText()).trim() === word) {
          await chip.click();
          clicked = true;
          break;
        }
      }
      expect(clicked).toBe(true);
    }

    // 7) La verificación quedó OK (la app confirma el respaldo en el backend y sale del alta).
    const setup = await $('#setup');
    await browser.waitUntil(async () => !(await setup.isDisplayed()), {
      timeout: 20000,
      timeoutMsg: 'el alta no se cerró tras verificar el respaldo (¿la verificación falló?)',
    });

    // 8) Estamos en la billetera y el backend responde: la vista de billetera se ve y la versión está cargada.
    const walletView = await $('[data-testid="view-wallet"]');
    await walletView.waitForDisplayed({ timeout: 15000 });
    const ver = await $('[data-testid="ver-chip"]');
    expect((await ver.getText()).trim()).toMatch(/^v\d/);
  });
});
