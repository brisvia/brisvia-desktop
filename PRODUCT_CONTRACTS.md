# PRODUCT_CONTRACTS.md — what is true on each screen

What a unit test does not see: **whether the text that appears makes sense for whoever is seeing it**.

Fernando found a message offering to "create your wallet" on a screen that **only someone who already has
one ever sees**. No mechanical sweep detects it: the keys were complete, with no language mixing, all green.
You need to know **who sees that screen and when**. That is what this file writes down.

Each contract states what **must** be seen and what is **forbidden** to see. The forbidden part is the one
that matters: it is what nobody reviews.

---

## States and contracts

### No wallet (first launch)
- **Shown:** the onboarding (welcome → create/import → password → 12 words → verification).
- **FORBIDDEN:** the waiting screen, the unlock screen, any balance, the main wallet.
- **Note:** the backend decides based on the file on disk (`wallet_seed_on_disk`), not on whether the node
  responds. If the query fails, assume a wallet DOES exist (fail-closed): showing the onboarding to someone
  who already has one invites overwriting it.

### With a wallet, before 2026-08-01 15:00 UTC (waiting mode)
- **Shown:** the waiting screen, the countdown, the usable wallet.
- **FORBIDDEN:** any text inviting the user to CREATE a wallet (they already have one). ← *Fernando's bug*
- **FORBIDDEN:** the "Syncing" state (it confuses: there is no network yet). Use "Waiting for launch".
- **FORBIDDEN:** the mine button being active.

### With a wallet, after launch
- **Shown:** the wallet, mining enabled, the real network status.
- **FORBIDDEN:** "Connected" with zero peers. ← impossible state
- **FORBIDDEN:** saying it is mining if the worker is not running.

### Password
- **Single rule:** minimum **6** characters. The message and the backend say EXACTLY the same thing.
  ← *Fernando's bug: the screen said 8, the backend required 12*
- **FORBIDDEN:** the screen announcing a minimum different from what the backend enforces.
- The strength bar is a **suggestion**, never a requirement.

### Mining mode (1.0)
- **Shown:** "Solo" active. Pool and other pool **disabled**, with the reason written out.
- **FORBIDDEN:** the pool buttons reacting (`POOL_ENABLED = false` rules; the screen only reflects it).
- **FORBIDDEN:** the screen offering pool if the backend does not allow it. The config is an editable file:
  the screen alone is not enough as a lock.

### The node does not start
- **Shown:** a message stating what happened and what to do, in the user's language.
- **FORBIDDEN:** raw node text in English. ← *the user saw "node is not ready yet"*
- **FORBIDDEN:** getting stuck forever on "Preparing wallet" (that is what happened with the corrupted
  database).
- Full disk / locked datadir / permissions: **reported, NOT "repaired"** (repairing does not fix them and
  causes a loop).

---

## Official terms (a single word per concept)

| Concept | ES | EN | Do not use |
|---|---|---|---|
| Mining group | **pool** | **pool** | ~~grupo~~, ~~group~~ ← *Fernando's bug* |
| Coin | Brisvia / BRVA | Brisvia / BRVA | — |
| Recovery words | 12 palabras | 12 words | ~~seed~~, ~~semilla~~ (to the user) |
| Real network | red real / mainnet | real network | — |

---

## Sources of truth (where each piece of data comes from)

| Data | SINGLE source | Never |
|---|---|---|
| **Version** | `app_version()` → `CARGO_PKG_VERSION` → `runningVersion` | hand-written in HTML/JS, nor read from the DOM ← *real bug: it was read from the on-screen message* |
| **Network** | the backend (`netcfg`) | inferred in the frontend |
| **Mining mode** | the backend (`POOL_ENABLED` + `mining_mode`) | whatever localStorage says |
| **Wallet exists** | the file on disk | whether the node responds |
| **Peers / height / difficulty** | the node via RPC | cached in the frontend |
| **Accepted share** | the pool's explicit confirmation | having sent it |

**General rule:** the DOM is never a source of state. The screen shows what the backend says; it does not
decide it.

---

## Formatting rules

| | ES | EN |
|---|---|---|
| Decimals | comma (`0,00`) | dot (`0.00`) |
| Launch date | 1 de agosto de 2026, 15:00 UTC (12:00 en Argentina) | August 1, 2026 at 15:00 UTC |
| Units (`H/s`) | same in both | same in both |

The language buttons each go **in their own language** ("Español" / "English"): that is correct, not a mix.
