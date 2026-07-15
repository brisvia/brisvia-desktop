# PROJECT_STATE

Single source of truth for "where Brisvia stands right now". No secrets, no transcripts.
Rewrite in place — this file is a snapshot, not a log.

**Updated:** 2026-07-15 · **Launch:** 2026-08-01 15:00 UTC (17 days out)

## Version

| | |
|---|---|
| Version | **1.0.5** (`Cargo.toml` == `tauri.conf.json`, verified) |
| Commit | `8a7fa71` |
| Published installers | Windows / macOS / Linux, all built `--features mainnet` |
| Genesis | `aa6bc268339aa9f4f2e39ae33aca7b7e48e395033d08d37c08f828890af7baf7` |
| Genesis time | `1785596400` = the launch instant itself |
| Installer SHA-256 | Win `6025b4af8186ca922de83db4b208cda840a37561a3cdecec702df067a5ad6266` |
| | macOS `9d5b041e9f5d5d0a26145ca9c8db3c8e501ff8fdbb1bd8fdc5c03eedbf224d77` |
| | Linux `28ddf0e2d4192c84a959df9565b88b0067cd5b0b8d07964e6731aa408bf5a9c0` |

## Open P0

**One, and it is not code: 1.0.6 must ship.** The two wallet P0s below are fixed *in the working tree* and
**not in the 1.0.5 the public has installed right now**. That version can still overwrite a phrase and can
still pay twice on a double click. ChatGPT's framing, agreed: *"si las correcciones P0 de billetera todavía
no están en la 1.0.5 pública, entonces 1.0.6 ya es obligatoria por billetera."*
**Blocked on Fernando's explicit authorisation — the mandate's one legitimate stop.**

Closed in the working tree after ChatGPT's cross-audit **overruled my decision to defer them**:

- **Phrase could be overwritten** — I had proved a wallet cannot be overwritten today (real regtest node:
  second `createwallet brisvia` → `error -4 "Database already exists."`, the `?` cuts, `encrypt_phrase_file`
  never runs) and filed the rest as an accepted risk. ChatGPT's counter was right and I was wrong: the
  irreplaceable asset was guarded by two assumptions *the overwriting function does not control*, and
  **"the frozen scope does not apply: this is not a new feature, it is reinforcing a critical invariant."**
  Fixed: `wallet_ops` exclusive guard + fail-closed check **before touching Core** + `encrypt_phrase_file_ex`
  refusing to overwrite (migration passes `allow_overwrite: true` on purpose).
- **Double click = two real payments** — `wallet_send` had no mutex. The frontend disables the button, but
  that is UX, not a barrier. Fixed: `sending` single-flight guard via **`try_lock`, never `lock()`** —
  queueing the second send would just pay twice a moment later. Loser gets `ERR:SEND_IN_PROGRESS`.

## Open P1

- **Ambiguous send timeout** — no reconciliation on a lost reply. `ERR:SEND_STATUS_UNKNOWN` and its ES/EN
  text exist; the backend does not yet distinguish a timeout from a refusal. Launch policy stands: **never
  auto-retry**. Last item of 1.0.6 Phase 1.

Closed this round:

- **Money crossed the IPC boundary as `f64`.** Now the raw string the user typed travels, the backend parses
  it into integer base units (briv), and the node gets an exact decimal string built from that integer — its
  RPC layer reads it with `ParseFixedPoint` (verified in `rpc/util.cpp`: it accepts `isStr()`). **No float
  touches an amount on the send path.** `amount_tests`: 11 tests on the boundaries ChatGPT listed —
  `0.1` is exactly 10,000,000 briv (not f64's 0.1000000000000000055 — this is the everyday reason, and it
  bites at ordinary amounts), 9 decimals **refused not rounded**, `1e-8` refused, cap at 100,000,000 BRVA.
  Precision note (an earlier version of this file got it wrong): 100,000,000 *is* exactly representable in
  an f64; what is not is the cap **in base units**, 1e16 briv > 2^53 ≈ 9.007e15, where consecutive briv
  collapse onto the same double.
- **`friendly_error` no longer leaks raw text.** Unknown node messages now return `ERR:OPERATION_FAILED`;
  the detail goes to the log with personal paths stripped (`sanitize_for_log`). Also added: `ERR:INVALID_AMOUNT`
  / `ERR:AMOUNT_TOO_SMALL` (a 9-decimal typo used to surface Core's raw "Invalid amount" **on the send
  screen**), ordered **before** the address rule since both start with "invalid", and `ERR:WALLET_LOCKED`
  (was falling through to "check the node is ready" — useless advice for someone who just needs to unlock).
- **Error contract swept and enforced:** `tools/check_error_sweep.py` — 35 backend codes, all mapped, all
  with text in both languages, and it **fails** if anyone restores the raw-text fallback (metatested).

- **Wallet errors reached the user as raw Core English.** `friendly_error` existed with the exact cases;
  9 of 10 wallet RPC calls never called it — including **both send paths**. The frontend maps `ERR:CODE`
  and passes unknown strings through unchanged (`app.js transError`), so users hit "Insufficient funds",
  "Invalid **Bitcoin** address", and a Windows path with their home directory in it, on money screens.
  Fixed: `friendly_error` now applied to send (both paths), restore, new address, encrypt, backup and
  tx detail. **Evidence:** `error_translation_tests`, 5 tests built from strings *captured from a real
  bitcoind*, plus a metatest proving the guard goes red when the rule is broken.

## Accepted risks

- **Ambiguous send timeout.** If Core broadcasts but the reply is lost, the app reports failure and a user
  may send again. The guard prevents *concurrent* duplicates, not this. Policy for launch (ChatGPT's, agreed):
  **never auto-retry `sendtoaddress`**. Full idempotency would mean rewriting the send path on PSBT — too
  large before Aug 1. Open P1, tracked below.
- **Amounts cross the boundary as `f64`** (`wallet_send(amount: f64)`). Not a launch blocker at current
  scale but wrong for money. Post-launch: string decimal or integer base units.
- **Pool is off at source** (`POOL_ENABLED = false`). Deliberate: the UI cannot yet show whether shares are
  accepted. See `PRODUCT_CONTRACTS.md`.
- **Private testnet is still running** until 2026-07-20 ART (`brisvia-testnet-stop.timer`, verified on the
  VPS). It is **not public** and must not be announced. Released installers are mainnet-only and cannot
  reach it.

## Evidence from the last closed block

**Block 1 — Wallet and funds** (2026-07-15)

- `wallet_key_tests`: **4 tests, 0 failed, 95.2 s**. Route `84h/9339h/0h` ✓ · address `brv1qtulvfeste55psz…` ✓
  · 5000 fresh wallets, 0 collisions ✓ — closes points 2, 3, 4, 5.
- `encrypt_phrase_file` writes atomically (temp + rename, owner-only perms on unix) — closes point 10.
- Overwrite protection verified against a real node — closes point 9.
- `error_translation_tests`: **5 tests** from captured Core strings + metatest red/green.
- `send_single_flight_tests`: **3 tests** — two real threads on a barrier prove the node is asked to send
  **exactly once**; the loser is *rejected, not queued* (asserted at <50 ms); the guard frees on failure.
- `phrase_never_overwritten_tests`: **3 tests** — a second wallet gets `ERR:WALLET_EXISTS` and the old file
  is **byte-for-byte unchanged**; the old phrase still decrypts with the old password.
- Both guards metatested: sabotage them and the tests go red ("the node was asked to send more than once").
- **Full battery: 37 tests, 0 failed, 103.3 s.** No regressions.
- Test node stopped, throwaway datadir deleted, no orphan `bitcoind`.

**ChatGPT's cross-audit** (12.760 chars, read in full from `output/raw/`) also claimed `walletlock` is
skipped when a send fails. **Disproved against the code**: `res` is captured without `?`, `walletlock` runs,
and the `?` comes after — the finally semantics are already there. It said so itself: *"no tengo delante el
archivo wallet_send completo"*. Read it all, verify each claim.

**Caught while fixing:** unifying the password rule into `friendly_error` as
`contains("passphrase") || contains("incorrect")` translated **both** of Core's passphrase messages —
so a user with a merely *locked* wallet would be told their password was wrong. Only capturing the real
strings exposed it; an invented test message would have passed green. The rule now requires both words,
and `a_locked_wallet_is_never_reported_as_a_wrong_password` guards it.

## Not yet verified in Block 1 (carried forward)

Points 7, 11, 13, 14 of the block need a running node + a real UI run, not unit tests:
compatibility with wallets from older versions · update without wallet loss · send/receive/fee/invalid
address end to end · double-click cannot duplicate a send. These belong to the E2E pass, not here.

## Block 2 — Node and network (in progress)

**Closed with evidence:**

- **`IsTestChain` audited at all four real call sites** (+ its own guard test). `BRISVIA_MAIN` is correctly
  NOT a test chain: `mempool_args` refuses `-acceptnonstdtxn` on mainnet, `versionbits` keeps the 95%
  threshold, and `walletutil` derives at `/0h` (not the testnet `/1h`).
- **RPC is not publicly exposed:** all three seeds run `rpcbind=127.0.0.1` + `rpcallowip=127.0.0.1`.
- **Launch timers armed to the exact instant** on all three: `Sat 2026-08-01 15:00:00 UTC`. NTP active and
  synchronised, UTC, on all three. Mainnet port 9333 correctly closed today (refused, not timing out).
- **Genesis of the published 1.0.5 is correct.** Scare investigated and dismissed: the local
  `src-tauri/binaries/bitcoind.exe` is a **9 July leftover** carrying the pre-recalibration genesis
  (`7f1cf9cf…`, nonce 90424, bits `0x1e7fffff`). It is gitignored and never shipped — CI compiles the node
  from source. `origin/main` carries `aa6bc268…` (nonce 79118, `0x1e0fffff`) since `e821a71`
  (14 Jul 04:07 UTC), and 1.0.5 was built 20 h later. Proven from the artifact, not inferred: the
  **published `.deb` was downloaded and its 280 MB `bitcoind` extracted** — it carries the two mainnet
  seeds that exist only on main.

**P1 high — the third seed node is missing from the shipped binary.** Mainnet has **no DNS seed**
(`vSeeds.clear()`), so `vFixedSeeds` is the only way a fresh client finds anyone. Verified inside the
published artifact: Hostinger ✓, Oracle-1 ✓, **Oracle-2 `129.159.108.102` ✗**. One of three prepared seed
nodes is unreachable by any new client. Fixed in the working tree (third seed added + a guard test that
**decodes the compiled list** and demands exactly those three on port 9333, no duplicates, nothing from
testnet). Ships with 1.0.6.

ChatGPT rated it P1, not P0 — two entry points on two different providers survive one failure — but
**"no aceptaría llegar al 1 de agosto sabiendo que uno de los tres nodos preparados no puede ser usado
directamente por un cliente nuevo."** It also correctly demolished my mitigation reasoning: my gossip
theory (the other two propagate Oracle-2 via `addr`) is **"una hipótesis plausible, no evidencia"** — addr
relay is selective and probabilistic. A DNS seed stood up now would **not** help 1.0.5 (`vSeeds` is empty
*in the binary* — my reading, confirmed). Distributing a `peers.dat` is rejected.

**Still open in Block 2:** prove or disprove the gossip path end to end · fresh-client bootstrap through
each seed individually (firewalled) · client started *before* the seeds and connecting without a restart ·
public IP announcement (`externalip` on Oracle?) · inbound capacity · network partition · inherited bans.

## Next mandatory block

Finish Block 2, then **Block 4 (installers/updater)** — because 1.0.6 has to go through the full gate.
