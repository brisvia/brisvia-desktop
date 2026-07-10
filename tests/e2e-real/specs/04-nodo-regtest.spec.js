// Recorrido P0 #4 — Nodo regtest real: la app levanta el nodo y muestra red y altura.
// Sobre la app COMPILADA real (backend Rust + bitcoind regtest efímero), verifica que:
//   - el backend levanta el nodo de verdad: el RPC responde (waitRpcUp);
//   - es regtest aislado (getblockchaininfo -> chain == regtest);
//   - la app muestra en el panel de red la RED y la ALTURA reales del nodo (nr-network, nr-height).
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Recorrido 4 — nodo regtest', () => {
  it('levanta el nodo, el RPC responde y la app muestra red y altura', async () => {
    const run = harness.fromEnv();

    // 1) Entrar a la app (creando billetera) para llegar al panel de red de la vista billetera.
    await harness.onboardCreate(PASSWORD);

    // 2) El backend levantó el nodo: el RPC responde y es regtest aislado.
    await harness.waitRpcUp(run.datadir, run.port, 60000);
    const chain = harness.rpc(run.datadir, run.port, ['getblockchaininfo']).stdout;
    expect(chain).toContain('"chain": "regtest"');

    // 3) La app muestra la RED (etiqueta no vacía) y la ALTURA que coincide con la del nodo.
    await (await $('.nav-btn[data-view="wallet"]')).click();
    const netEl = await $('[data-testid="nr-network"]');
    await browser.waitUntil(async () => {
      const t = (await netEl.getText()).trim();
      return t.length > 0 && t !== '—';
    }, { timeout: 20000, timeoutMsg: 'el panel de red nunca mostró la red' });

    const nodeHeight = harness.blockCount(run.datadir, run.port); // 0 en génesis regtest
    const heightEl = await $('[data-testid="nr-height"]');
    await browser.waitUntil(async () => {
      const t = (await heightEl.getText()).trim();
      return /^\d[\d.,]*$/.test(t) && parseInt(t.replace(/[.,]/g, ''), 10) === nodeHeight;
    }, { timeout: 20000, timeoutMsg: `la altura mostrada no coincidió con la del nodo (${nodeHeight})` });
  });
});
