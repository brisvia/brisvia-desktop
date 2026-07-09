# Brisvia Desktop

All-in-one desktop wallet and CPU miner for [Brisvia](https://brisvia.com) (BRVA), a transparent,
CPU-mineable Proof-of-Work cryptocurrency. One app to hold a wallet and mine BRVA on an ordinary computer,
with no command line required.

The coin itself (the node and consensus code) lives in the core repository:
[github.com/brisvia/brisvia](https://github.com/brisvia/brisvia).

## Features

- **Wallet** — create or restore a wallet from a 12-word BIP39 backup phrase, send and receive BRVA, and
  view your balance and history.
- **One-click mining** — mine with RandomX using your CPU, with adjustable intensity (light / balanced / full).
- **Bundled node** — ships with a Brisvia node and mining engine; it connects to the shared network,
  syncs and validates on its own.
- **Bilingual** — the interface is available in Spanish and English, and follows the system language by default.

## Download

Ready-to-run installers for each release are on the
[Releases page](https://github.com/brisvia/brisvia-desktop/releases/latest):

- **Windows** — `Brisvia-Miner-Windows.exe` (installer)
- **macOS** — `Brisvia-Miner-macOS.dmg`

After installing, open Brisvia Miner, create or restore a wallet from your 12 words, and press **Start** to
mine — no configuration and no command line required. The app updates itself when a new signed version is
available. Learn more at [brisvia.com](https://brisvia.com).

## How it works

Brisvia Desktop is built with [Tauri](https://tauri.app): a Rust backend (`src-tauri`) drives the system
webview UI (`src/renderer`), and orchestrates two bundled native processes — the Brisvia node (`bitcoind`)
and the RandomX mining engine. The RandomX worker lives in `crates/brisvia-randomx`.

## Status

This is early, experimental software running against the Brisvia **test network**. Test coins have no
monetary value. Always keep your 12 backup words safe and never share your private keys.

## Building from source

Requires [Rust](https://www.rust-lang.org/tools/install) and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform.

    # install the frontend/build tooling
    npm install

    # run in development
    npm run tauri dev

    # build a release bundle (installer/app for the current OS)
    npm run tauri build

The Brisvia node and mining-engine binaries are produced from the core repository and placed under
`src-tauri/binaries/` before bundling; they are not checked into this repository.

## License

MIT. See [LICENSE](LICENSE).
