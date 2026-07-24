# -*- coding: utf-8 -*-
"""Release identity guard: everything that defines "this is the real Brisvia network".

Why it exists: each of these values, if it goes wrong, produces a SILENT failure that is only found on
launch day, when people already have the program installed. A wrong network, a wallet derived on another
branch, a pool that turns itself on, or mining enabled ahead of time do not raise an error: they simply do
something else. This script gathers them all in one place and fails if any of them moved.

Runs WITHOUT building: it reads the sources. Verification against the already-built binary is separate
(the workflows extract the sidecar and run it).

Usage:  python tools/check_release_identity.py [--version 1.0.6]
"""
import argparse
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
# The core to verify must be the SAME network CI compiles (brisvia/brisvia pinned to the 9342 port move).
# Locally there can be two checkouts side by side: the canonical one (…/dev/cripto-pow, already on 9342) and an
# older sibling copy that still carries Litecoin's 9333. Prefer the first that actually has the chain sources,
# looking at the canonical location first, so the pre-push guard never validates against a stale port.
_CORE_CANDIDATES = [ROOT.parents[1] / "cripto-pow", ROOT.parent / "cripto-pow"]
CORE = next((c for c in _CORE_CANDIDATES if (c / "src" / "kernel" / "chainparams.cpp").exists()), _CORE_CANDIDATES[-1])

# The canonical values of the real network. If one changes on purpose, it changes HERE, in the same commit,
# with the reason. This script going green on its own does not mean the change is correct.
GENESIS = "aa6bc268339aa9f4f2e39ae33aca7b7e48e395033d08d37c08f828890af7baf7"
GENESIS_TIME = "1785596400"   # 2026-08-01 15:00 UTC: the launch instant
MAINNET_START = "1_785_596_400"
COIN_TYPE = "9339"
HRP = "brv"
P2P_PORT = "9342"
# The exact frozen version this release ships the pool ON with. A future pool-enabled release must bump this
# HERE, in the same commit, with the reason. It is NOT ">=": a future version does not silently inherit the
# authorisation to enable the pool — that is the whole point of pinning it.
POOL_RELEASE_VERSION = "1.1.1"
SEEDS = ["187.77.240.145:9342", "129.80.250.36:9342", "129.159.108.102:9342"]


def read(p: pathlib.Path) -> str:
    return p.read_text(encoding="utf-8", errors="replace")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", help="expected version (e.g. 1.0.6). If omitted, only checks that the sources agree with each other.")
    args = ap.parse_args()

    failures, oks = [], []

    def check(cond, ok_msg, fail_msg):
        (oks if cond else failures).append(ok_msg if cond else fail_msg)

    # ---- Version: the three manifests that must agree, exactly ----
    # Cargo.toml drives the sidecar, tauri.conf.json drives the installer + updater, package.json drives the
    # npm build. If any one disagrees, the updater offers a version the installer did not ship. All three, or fail.
    cargo = read(ROOT / "src-tauri/Cargo.toml")
    tauri = read(ROOT / "src-tauri/tauri.conf.json")
    pkg = read(ROOT / "package.json")
    v_cargo = re.search(r'^version = "([0-9.]+)"', cargo, re.M)
    v_tauri = re.search(r'"version": "([0-9.]+)"', tauri)
    v_pkg = re.search(r'"version":\s*"([0-9.]+)"', pkg)
    check(v_cargo and v_tauri and v_pkg, "version readable in the three manifests",
             "could not read the version in Cargo.toml / tauri.conf.json / package.json")
    rel = v_cargo.group(1) if v_cargo else None
    if v_cargo and v_tauri and v_pkg:
        check(v_cargo.group(1) == v_tauri.group(1) == v_pkg.group(1),
                 f"version in sync across the three manifests ({v_cargo.group(1)})",
                 f"VERSION OUT OF SYNC: Cargo.toml={v_cargo.group(1)} tauri.conf.json={v_tauri.group(1)} "
                 f"package.json={v_pkg.group(1)} -> the updater would offer a different version than the installer ships")
        if args.version:
            check(v_cargo.group(1) == args.version,
                     f"version is the expected one ({args.version})",
                     f"the version is {v_cargo.group(1)} but {args.version} was expected")

    # ---- Network identity in the miner ----
    lib = read(ROOT / "src-tauri/src/lib.rs")
    check(f'const MAINNET_START: i64 = {MAINNET_START};' in lib,
             "MAINNET_START is the launch instant",
             f"MAINNET_START is not {MAINNET_START}: the program would wait for a different time")
    check(re.search(r'pub const NET_CHAIN: &str = "brisvia";', lib) is not None,
             "the real network is named brisvia",
             "NET_CHAIN=brisvia not found in the mainnet config")
    # Pool: from 1.0.9 the candidate ships the pool ENABLED (honest share UI is in). The guard is not removed —
    # it is INVERTED and pinned: POOL_ENABLED must be declared EXACTLY ONCE as true (never off, never ambiguous,
    # never twice) AND the release version must be EXACTLY the pinned one. A build can never ship the pool
    # half-wired: not with an old version, not with a future version that never went through this gate.
    pool_on_n = lib.count('const POOL_ENABLED: bool = true;')
    pool_off_n = lib.count('const POOL_ENABLED: bool = false;')
    check(pool_on_n == 1 and pool_off_n == 0,
             "POOL_ENABLED is declared exactly once and is true",
             f"POOL_ENABLED must appear exactly once as 'true' (found true x{pool_on_n}, false x{pool_off_n})")
    check(rel == POOL_RELEASE_VERSION,
             f"pool-enabled build carries the pinned release version {POOL_RELEASE_VERSION}",
             f"POOL_ENABLED=true is only authorised for exactly {POOL_RELEASE_VERSION}, but the version is {rel!r}: "
             f"an old version cannot ship the pool, and a future version must bump POOL_RELEASE_VERSION here first")
    check(re.search(r'"84h/' + COIN_TYPE + r"h/0h\"|84h/9339h/0h", lib) is not None or "9339" in lib,
             f"coin type {COIN_TYPE} present",
             f"coin type {COIN_TYPE} not found: the wallet would derive on another branch")

    # ---- Network identity in the core ----
    if CORE.exists():
        chain = read(CORE / "src/kernel/chainparams.cpp")
        main_blk = chain[chain.find("CBrisviaMainParams()"):]
        main_blk = main_blk[: main_blk.find("class C", 10) if main_blk.find("class C", 10) > 0 else 22000]
        check(GENESIS in main_blk, "real network genesis correct",
                 f"the mainnet genesis is NOT {GENESIS[:16]}...: it would be a different chain")
        check(f"genesisTime = {GENESIS_TIME}" in main_blk,
                 "the genesis is dated at the launch instant",
                 f"genesisTime is not {GENESIS_TIME}")
        check(f'nDefaultPort = {P2P_PORT};' in main_blk, f"P2P port {P2P_PORT}",
                 f"the mainnet P2P port is not {P2P_PORT}")
        check(f'bech32_hrp = "{HRP}"' in main_blk, f"{HRP}1... addresses",
                 f'the address prefix is not "{HRP}"')
        check("vSeeds.clear()" in main_blk,
                 "no DNS seeds (documented: bootstrap depends on the fixed ones)",
                 "vSeeds changed: review, bootstrap depends on this")

        # The fixed seeds, decoded from the array (not from the comment).
        seeds_h = read(CORE / "src/chainparamsseeds.h")
        m = re.search(r"chainparams_seed_brisvia_main\[\]\s*=\s*\{(.*?)\};", seeds_h, re.S)
        if not m:
            failures.append("mainnet fixed-seed array not found")
        else:
            b = [int(x, 16) for x in re.findall(r"0x([0-9a-fA-F]{2})", m.group(1))]
            got = []
            for i in range(0, len(b) - 7, 8):
                e = b[i:i + 8]
                if e[0] == 0x01 and e[1] == 0x04:
                    got.append(f"{e[2]}.{e[3]}.{e[4]}.{e[5]}:{(e[6] << 8) | e[7]}")
            for s in SEEDS:
                check(s in got, f"seed {s} compiled in",
                         f"MISSING seed {s}: no freshly installed program could find that node")
            check(len(got) == len(SEEDS), f"exactly {len(SEEDS)} seeds",
                     f"there are {len(got)} seeds and there should be {len(SEEDS)}: {got}")
    else:
        failures.append(f"core not found at {CORE}: cannot verify genesis or seeds")

    # ---- Updater key: guard against an accidental change ----
    pub = re.search(r'"pubkey"\s*:\s*"([^"]+)"', tauri)
    check(pub is not None and len(pub.group(1)) > 40,
             "updater public key present",
             "updater public key not found: updates would not be verified")

    print(f"RELEASE IDENTITY — {len(oks)} OK, {len(failures)} failures\n")
    for o in oks:
        print(f"  OK    {o}")
    if failures:
        print()
        for f in failures:
            print(f"  FAIL  {f}")
        print("\nDO NOT FREEZE OR BUILD UNTIL THIS IS RESOLVED.")
        return 1
    print("\nOK: the real network identity is complete and coherent.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
