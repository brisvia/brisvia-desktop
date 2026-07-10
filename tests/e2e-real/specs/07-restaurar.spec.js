// Recorrido P0 #7 — Restaurar billetera desde 12 palabras (flujo real de "cambiar de PC").
// Sobre la app COMPILADA real (backend Rust real, sin depender del nodo), verifica que:
//   - el alta permite elegir "importar" y muestra la grilla de 12 casillas;
//   - se pueden tipear 12 palabras válidas (frase estándar BIP39) y avanzar;
//   - se pide una contraseña nueva para cifrar la billetera restaurada;
//   - el backend restaura de verdad (wallet.restore) y sale del alta mostrando la billetera con dirección.
// Es "backend real sin nodo": restaurar no depende de que el nodo esté arriba.
'use strict';

const harness = require('../helpers/harness');

// Frase de prueba canónica BIP39 (entropía cero, checksum válido). El backend usa el crate `bip39`
// con wordlist inglés estándar, así que la acepta. Es pública y de prueba: nunca tiene fondos reales.
const SEED = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const PASSWORD = 'brisvia-e2e-1234';

describe('Recorrido 7 — restaurar billetera', () => {
  it('importa 12 palabras válidas, pide contraseña y entra a la billetera con dirección', async () => {
    harness.fromEnv();

    // 1) Bienvenida -> elegir "importar" -> aparece la grilla de 12 casillas.
    await harness.skipWelcome();
    const importBtn = await $('#btn-import');
    await importBtn.waitForClickable({ timeout: 10000 });
    await importBtn.click();

    const importGrid = await $('#import-grid');
    await importGrid.waitForDisplayed({ timeout: 10000 });
    await browser.waitUntil(async () => (await importGrid.$$('input')).length === 12, {
      timeout: 10000, timeoutMsg: 'la grilla de importar no mostró 12 casillas',
    });

    // 2) Tipear las 12 palabras (una por casilla, en orden).
    const words = SEED.split(' ');
    const inputs = await importGrid.$$('input');
    for (let i = 0; i < 12; i++) await inputs[i].setValue(words[i]);

    // 3) Confirmar la frase -> paso de contraseña.
    await (await $('#import-ok')).click();
    const pass = await $('[data-testid="onb-pass"]');
    await pass.waitForDisplayed({ timeout: 10000 });
    await (await $('[data-testid="pass-1"]')).setValue(PASSWORD);
    await (await $('[data-testid="pass-2"]')).setValue(PASSWORD);
    await (await $('[data-testid="pass-next"]')).click();

    // 4) El backend restaura y sale del alta: la vista de billetera aparece y la versión está cargada.
    const setup = await $('#setup');
    await browser.waitUntil(async () => !(await setup.isDisplayed()), {
      timeout: 30000, timeoutMsg: 'el alta no se cerró tras restaurar (¿wallet.restore falló?)',
    });
    const walletView = await $('[data-testid="view-wallet"]');
    await walletView.waitForDisplayed({ timeout: 15000 });
    expect((await (await $('[data-testid="ver-chip"]')).getText()).trim()).toMatch(/^v\d/);

    // 5) La billetera restaurada da una dirección real para recibir.
    const addr = await harness.readReceiveAddress();
    expect(addr.length).toBeGreaterThan(10);
  });
});
