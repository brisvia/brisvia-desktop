// Recorrido P0 #10 — Enviar: modal, "usar máximo" y validaciones.
// Sobre la app COMPILADA real, verifica el modal de enviar SIN llegar a mandar dinero:
//   - abre el modal;
//   - una dirección vacía/inválida da error ("dirección inválida");
//   - con dirección válida pero monto 0/negativo da error ("monto inválido");
//   - "usar máximo" completa el monto con el saldo disponible.
// No ejecuta un envío real (la billetera nueva no tiene fondos): sólo valida la UX de entrada.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Recorrido 10 — enviar (validaciones)', () => {
  it('valida dirección y monto y completa el máximo', async () => {
    harness.fromEnv();

    await harness.onboardCreate(PASSWORD);

    // Abrir el modal de enviar desde la billetera.
    await (await $('.nav-btn[data-view="wallet"]')).click();
    await (await $('#act-send')).click();
    const sendModal = await $('#modal-send');
    await sendModal.waitForDisplayed({ timeout: 10000 });

    const addr = await $('#send-addr');
    const amount = await $('#send-amount');
    const go = await $('#send-go');
    const msg = await $('#send-msg');

    // 1) Dirección inválida -> error de dirección.
    await addr.setValue('no-es-una-direccion');
    await amount.setValue('1');
    await go.click();
    await browser.waitUntil(async () => await msg.isDisplayed() && (await msg.getText()).trim().length > 0, {
      timeout: 8000, timeoutMsg: 'no rechazó la dirección inválida',
    });
    const errAddr = (await msg.getText()).trim();
    expect(errAddr.length).toBeGreaterThan(0);

    // 2) Dirección con formato válido (contiene "brv" y suficiente largo) pero monto 0 -> error de monto.
    await addr.setValue('brv1qexampleexampleexampleexample00');
    await amount.setValue('0');
    await go.click();
    await browser.waitUntil(async () => {
      const t = (await msg.getText()).trim();
      return await msg.isDisplayed() && t.length > 0 && t !== errAddr;
    }, { timeout: 8000, timeoutMsg: 'no rechazó el monto inválido' });

    // 3) "Usar máximo" completa el monto con el saldo disponible (0 en billetera nueva, pero completa el campo).
    await (await $('#send-max')).click();
    await browser.waitUntil(async () => (await amount.getValue()).trim().length > 0, {
      timeout: 5000, timeoutMsg: 'el botón usar máximo no completó el monto',
    });

    // Cerrar el modal.
    await (await sendModal.$('[data-close]')).click();
  });
});
