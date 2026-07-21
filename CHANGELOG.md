# Changelog

What changed in each published release of Brisvia Desktop. Installers and checksums for every version are on
the [Releases page](https://github.com/brisvia/brisvia-desktop/releases).

Versions before 1.0.7 were pre-release builds and are not listed.

## 1.1.0 — 2026-07-21

- **Five languages.** Added Portuguese (Brazil), Simplified Chinese and Russian alongside English and
  Spanish. On first run the app follows the operating system's language; a picker in the header changes it
  at any time.
- **Clearer failures when something blocks the app.** When Brisvia cannot reach one of its own bundled
  components, it now says so and names the likely cause — a security program that blocked or quarantined
  it — instead of falling back to a generic "the operation could not be completed". The message for a
  missing component now points at quarantine rather than blaming the download. Both in all five languages.
  Someone spent an evening writing firewall rules for a program the firewall had never blocked; this is the
  fix for that.
- **Amounts are written the way each language writes them.** One shared rule decides which languages use a
  decimal comma, so the same number is never rendered two different ways inside the app.
- **The launch is announced in UTC only.** It previously added "12:00 Argentina time", which means nothing
  to a reader in China or Russia.
- Fixed a small vertical scroll on the mining view.

## 1.0.9 — 2026-07-20

- **Mining unlocks by itself at the mainnet launch**, with opt-in auto-start for people who want the app to
  begin mining when it opens.
- **Share button** on the mining screen, with hardened URL opening.
- Official mining pool enabled for the mainnet launch.
- Release pipeline: corrected the seed-node port check (the binary ships P2P 9342) and retargeted the
  update-migration gate to 1.0.8 → 1.0.9.

## 1.0.8 — 2026-07-17

- Node source pinned to the mainnet-ports core commit: **P2P 9342, RPC 9338**, moving off ports that
  collided with another network.
- Added a headless Linux job that proves a wallet survives an update: it creates a throwaway wallet, runs
  the old version, updates to the new one, and verifies the wallet is still there.
- Several fixes to the automated test suite, including a non-ASCII installation path.

## 1.0.7 — 2026-07-17

- First public release.
- UX improvements to the mining view.
- The send screen now says "estimated total to debit": the real fee can differ from the estimate, and the
  wording should not promise otherwise.
- Release pipeline: check out by commit rather than by tag (a draft release's tag does not exist in git
  yet), and run the node step under bash on every platform.
