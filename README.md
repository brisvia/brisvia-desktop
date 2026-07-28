# Brisvia Desktop

All-in-one desktop wallet and CPU miner for [Brisvia](https://brisvia.com) (BRVA), a transparent,
CPU-mineable Proof-of-Work cryptocurrency with a fair launch. One app to hold a wallet and mine BRVA on an
ordinary computer, with no command line required.

The coin itself (the node and consensus code) lives in the core repository:
[github.com/brisvia/brisvia](https://github.com/brisvia/brisvia).

## Status

The main network launches on **August 1, 2026 at 15:00 UTC**. Until then the app installs and runs, and you
can create your wallet and leave it ready, but there is no real chain to mine yet.

There is **no premine, no presale and no developer allocation**: the first block is mined by whoever is
running at launch, the same as everyone else. Emission is 50 BRVA per block, halving every 1,000,000 blocks,
capped at 100,000,000 BRVA.

This is young software. It has an automated test suite and every release is verified before publication (see
[Releases and verification](#releases-and-verification)), but it has not been through a third-party security
audit. Keep your 12 backup words somewhere safe and offline, and never share them or your private keys with
anyone — including anyone claiming to be from Brisvia.

## Features

- **Wallet** — create or restore a wallet from a 12-word BIP39 backup phrase, send and receive BRVA, and
  view your balance and history. The words never leave your machine.
- **One-click mining** — mine with RandomX using your CPU, with adjustable intensity (light / balanced / full).
  Mining is off until you start it, and one button stops it.
- **Bundled node** — ships with a Brisvia node and mining engine; it connects to the network, syncs and
  validates on its own.
- **Five languages** — English, Spanish, Portuguese (Brazil), Simplified Chinese and Russian. On first run
  it follows your system language; you can change it any time from the globe in the header.
- **Signed automatic updates** — the app offers a new version only when its signature validates against the
  key built into the app.

## Download

Installers for the current release are on the
[Releases page](https://github.com/brisvia/brisvia-desktop/releases/latest), and are also linked from
[brisvia.com](https://brisvia.com) with their checksums:

| System  | File                                  | Notes                        |
|---------|---------------------------------------|------------------------------|
| Windows | `Brisvia-Miner-Windows.exe`           | Windows 10 and 11, 64-bit    |
| macOS   | `Brisvia-Miner-macOS.dmg`             | Apple Silicon                |
| Linux   | `Brisvia.Miner_<version>_amd64.deb`   | Debian and Ubuntu, 64-bit    |
| Linux   | `Brisvia.Miner_<version>_amd64.AppImage` | 64-bit — **see the warning below** |

> **On Ubuntu 24.04 or newer, take the `.deb`.** The 1.1.0 AppImage does not start there: it bundles an
> older GLib than those systems ship, and their GIO/GVFS modules — which are loaded from the system
> whatever the AppImage carries — call a function that older GLib does not export. Reproduced inside a
> real Ubuntu 26.04 container and fixed in the build for the next release. Ubuntu 22.04 and Debian 12
> are unaffected, and the `.deb` never had the problem because it uses the host's GLib.

After installing, open Brisvia Miner, save your 12 words, and press **Start**.

### Verify what you downloaded

Do not skip this. Compare the SHA-256 of your file against the one published next to the download on
[brisvia.com](https://brisvia.com). If they differ, delete the file and download again from that site only.

    # Windows (PowerShell)
    Get-FileHash .\Brisvia-Miner-Windows.exe -Algorithm SHA256

    # macOS
    shasum -a 256 Brisvia-Miner-macOS.dmg

    # Linux
    sha256sum Brisvia.Miner_*_amd64.AppImage

### If Windows or your antivirus blocks it

The executables are not code-signed yet, so Windows warns about an unknown publisher and Chrome may block
the download. Separately, Brisvia genuinely bundles a miner, and some antivirus products flag mining
software on sight — that is a fair call on their part, not a mistake. Brisvia never mines behind your back:
it ships switched off, shows exactly how much processor it is using, stops with one button, and uninstalls
cleanly.

[brisvia.com/blocked](https://brisvia.com/blocked/) walks through each warning, what it means, and how to get
past it safely — including why you should authorise the specific quarantined file rather than excluding the
whole folder.

## How it works

Brisvia Desktop is built with [Tauri](https://tauri.app): a Rust backend (`src-tauri`) drives the system
webview UI (`src/renderer`), and orchestrates two bundled native processes — the Brisvia node (`bitcoind`)
and the RandomX mining engine. The RandomX worker lives in `crates/brisvia-randomx`.

## Releases and verification

A release is not published by hand. `.github/workflows/publish-approved-release.yml` re-checks everything
before anything becomes public, and refuses to continue on any mismatch:

- the tag must point at the exact approved commit, and `main` must carry that same tree, so what ships can be
  rebuilt from this repository;
- the version in `src-tauri/tauri.conf.json` must match the version being published;
- a `release-go-v<version>.json` manifest must be present with every gate at `PASS`, and its SHA-256 must
  match the one passed to the workflow — so an edited manifest stops the release;
- the release must still be a draft, and the updater manifest must not already offer that version;
- **after** publishing, the workflow downloads every asset again without credentials, as a stranger would,
  and compares them against the manifest.

The three updater artifacts are signed with minisign (ed25519 over BLAKE2b-512) and the app installs an
update only if the signature validates against the public key embedded in `tauri.conf.json`.

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

## Security

Never share your 12 words. No one from Brisvia will ever ask for them, and there are no giveaways or
airdrops. To report a vulnerability, see [SECURITY.md](SECURITY.md).

## License

MIT. See [LICENSE](LICENSE).
