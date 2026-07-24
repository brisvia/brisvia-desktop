// P0 journey #16 — Changing the CPU intensity propagates to the auto-start summary (real-network build).
//
// The bug: the auto-start summary (#auto-start-cpu) captured the CPU preset at the moment the toggle was ARMED
// and did NOT update when the slider changed afterwards — it showed e.g. "Balanced" while the slider was on
// "High". The auto-start box exists ONLY on the mainnet build (renderAutoStart returns early when !isMainnetBuild),
// so this journey runs on APP_MAINNET_E2E with the clock frozen BEFORE launch (configured in run.js). The fix:
// setPower now repaints the summary from the active preset and re-captures the auto-start choice when armed.
'use strict';

const harness = require('../helpers/harness');

const PASSWORD = 'brisvia-e2e-1234';

describe('Journey 16 — CPU intensity propagates to the auto-start summary', () => {
  it('the summary follows the slider, not the value captured when the toggle was armed', async () => {
    harness.fromEnv();
    await harness.onboardCreate(PASSWORD);

    await (await $('.nav-btn[data-view="mine"]')).click();
    await (await $('[data-testid="view-mine"]')).waitForDisplayed({ timeout: 10000 });

    // The auto-start box only renders on the mainnet build. Arm it (this captures the current CPU preset).
    const toggle = await $('#auto-start-toggle');
    await toggle.waitForDisplayed({ timeout: 15000 });
    if (!(await toggle.isSelected())) await toggle.click();
    const cpuEl = await $('#auto-start-cpu');
    await cpuEl.waitForDisplayed({ timeout: 8000 });
    await browser.waitUntil(async () => (await cpuEl.getText()).trim().length > 0, {
      timeout: 8000, timeoutMsg: 'the auto-start CPU summary never rendered a value',
    });

    // Pick a preset whose label differs from what the summary currently shows, click it, and assert the summary
    // FOLLOWS the slider (equals the active preset button's label). Without the fix it stays on the captured value.
    async function pickPresetDifferentFromSummary() {
      const summary = (await cpuEl.getText()).trim();
      for (const pct of ['25', '50', '75', '100']) {
        const btn = await $(`.mine-grid .seg-btn[data-pct="${pct}"]`);
        if (await btn.isExisting()) {
          const label = (await btn.getText()).trim();
          if (label && label !== summary) { await btn.click(); return label; }
        }
      }
      return null;
    }
    const chosen = await pickPresetDifferentFromSummary();
    expect(chosen).toBeTruthy();

    await browser.waitUntil(async () => (await cpuEl.getText()).trim() === chosen, {
      timeout: 8000,
      timeoutMsg: `the auto-start summary stayed stale instead of following the slider to "${chosen}"`,
    });
  });
});
