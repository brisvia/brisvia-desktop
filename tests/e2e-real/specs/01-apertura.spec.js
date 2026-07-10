// Recorrido P0 #1 — Primera apertura.
// Verifica, sobre la app COMPILADA real (backend Rust + nodo regtest efímero), que:
//   - la app arranca y el frontend carga sus recursos (chip de versión lleno desde app_version del backend);
//   - el backend responde y levanta el nodo de verdad (el RPC del bitcoind regtest queda disponible);
//   - la app no queda en un error fatal: al detectar que no hay billetera, muestra el alta (onboarding);
//   - el nodo es realmente regtest y aislado (la redirección e2e funciona de punta a punta).
// (Que no queden procesos huérfanos se valida en el cierre de sesión, en afterSession del harness.)
'use strict';

const harness = require('../helpers/harness');

describe('Recorrido 1 — primera apertura', () => {
  it('arranca, el backend responde, levanta el nodo regtest y muestra el alta sin error fatal', async () => {
    const run = harness.fromEnv();

    // 1) El frontend cargó y habló con el backend: el chip de versión se llena desde app_version.
    const ver = await $('[data-testid="ver-chip"]');
    await ver.waitForExist({ timeout: 30000 });
    await browser.waitUntil(async () => (await ver.getText()).trim().length > 0, {
      timeout: 30000,
      timeoutMsg: 'el chip de versión nunca se llenó (¿el backend no respondió app_version?)',
    });
    expect((await ver.getText()).trim()).toMatch(/^v\d/);

    // 2) El backend levantó el nodo: el RPC del bitcoind regtest responde.
    await harness.waitRpcUp(run.datadir, run.port, 60000);

    // 3) Es regtest de verdad (la redirección e2e llegó hasta el nodo).
    const chain = harness.rpc(run.datadir, run.port, ['getblockchaininfo']).stdout;
    expect(chain).toContain('"chain": "regtest"');

    // 4) Sin billetera en disco -> la app decide mostrar el alta (onboarding), señal de arranque sano.
    const welcome = await $('[data-testid="onb-welcome"]');
    await welcome.waitForDisplayed({ timeout: 60000 });
    expect(await welcome.isDisplayed()).toBe(true);
  });
});
