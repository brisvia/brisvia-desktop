# Contributing

Brisvia Desktop is the wallet and miner people install. A bug here costs someone their coins or their
afternoon, so the bar is practical rather than ceremonial: explain what you changed and how you know it
works.

## Before you start

- **Consensus, networking and validation are not in this repository.** They live in
  [github.com/brisvia/brisvia](https://github.com/brisvia/brisvia). Changes to how blocks are validated,
  how difficulty adjusts, or how the network talks belong there.
- **Do not report security problems as issues.** See [SECURITY.md](SECURITY.md).
- For anything larger than a fix, open an issue first and describe the problem before writing the solution.

## Running it locally

Requires [Rust](https://www.rust-lang.org/tools/install) and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform.

    npm install
    npm run tauri dev

The bundled node and mining-engine binaries come from the core repository and are placed under
`src-tauri/binaries/`; they are not checked in here.

## Checks to run before opening a pull request

These are the same guards CI runs. Running them locally takes seconds; CI takes about twenty minutes per
platform.

    cargo test --manifest-path src-tauri/Cargo.toml            # app unit tests
    cargo test --manifest-path crates/brisvia-randomx/Cargo.toml  # mining engine
    node test_locales.cjs                    # every language has every key, with matching placeholders
    python tools/check_textos.py             # no untranslated strings left in the UI
    python tools/check_error_contract.py     # every error the backend can emit has a message in every language
    python tools/check_english_only.py       # nothing but English in the repository itself

## Conventions

- **Everything in this repository is written in English** — code, comments, commit messages, documentation.
  The user-facing translations live in `src/renderer/locales.js`.
- **User-facing text is written for someone who is not technical.** No jargon, no internal names. An error
  says what happened and what to do about it.
- When you add or change a user-facing string, add it in **all five languages**. `test_locales.cjs` fails if
  one is missing.
- Commit messages explain *why*, not just *what*. The diff already shows what changed.
- Keep the change small and focused on one thing.

## Releases

Releases are cut by the maintainer through `.github/workflows/publish-approved-release.yml`, which re-checks
the tag, the tree, the version, the signed manifest and the artifacts before and after publishing. Pull
requests do not need to touch version numbers, manifests or changelogs for a release — that happens as part
of the release itself.
