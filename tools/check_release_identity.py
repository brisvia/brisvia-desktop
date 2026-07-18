# -*- coding: utf-8 -*-
"""Release identity guard: everything that defines "this is the real Brisvia network".

Why it exists: each of these values, if it comes out wrong, produces a SILENT failure that is only
discovered on launch day, when people already have the program installed. A wrong network, a wallet
derived on another branch, a pool that turns itself on, or mining enabled ahead of time raise no error:
they simply do something else. This script puts them all in one place and fails if any of them moved.

Runs WITHOUT compiling: it reads the sources. Verification against the already-built binary is separate
(the workflows extract the sidecar and run it).

Usage:  python tools/check_release_identity.py [--version 1.0.6]
"""
import argparse
import pathlib
import re
import sys

RAIZ = pathlib.Path(__file__).resolve().parents[1]
CORE = RAIZ.parent / "cripto-pow"

# The canonical values of the real network. If any of them changes on purpose, it is changed HERE, in the
# same commit, with an explanation of why. This script going green on its own does not mean the change is
# correct.
GENESIS = "aa6bc268339aa9f4f2e39ae33aca7b7e48e395033d08d37c08f828890af7baf7"
GENESIS_TIME = "1785596400"   # 2026-08-01 15:00 UTC: the instant of the launch
MAINNET_START = "1_785_596_400"
COIN_TYPE = "9339"
HRP = "brv"
P2P_PORT = "9333"
SEEDS = ["187.77.240.145:9333", "129.80.250.36:9333", "129.159.108.102:9333"]


def leer(p: pathlib.Path) -> str:
    return p.read_text(encoding="utf-8", errors="replace")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", help="expected version (e.g. 1.0.6). If omitted, only checks that they match each other.")
    args = ap.parse_args()

    fallos, oks = [], []

    def chequear(cond, ok_msg, fail_msg):
        (oks if cond else fallos).append(ok_msg if cond else fail_msg)

    # ---- Version: the only two allowed sources ----
    cargo = leer(RAIZ / "src-tauri/Cargo.toml")
    tauri = leer(RAIZ / "src-tauri/tauri.conf.json")
    v_cargo = re.search(r'^version = "([0-9.]+)"', cargo, re.M)
    v_tauri = re.search(r'"version": "([0-9.]+)"', tauri)
    chequear(v_cargo and v_tauri, "version readable in both sources", "could not read the version")
    if v_cargo and v_tauri:
        chequear(v_cargo.group(1) == v_tauri.group(1),
                 f"version in sync ({v_cargo.group(1)})",
                 f"VERSION OUT OF SYNC: Cargo.toml={v_cargo.group(1)} vs tauri.conf.json={v_tauri.group(1)} "
                 f"-> the updater would offer a version different from the one the installer carries")
        if args.version:
            chequear(v_cargo.group(1) == args.version,
                     f"version is the expected one ({args.version})",
                     f"the version is {v_cargo.group(1)} and {args.version} was expected")

    # ---- Network identity in the miner ----
    lib = leer(RAIZ / "src-tauri/src/lib.rs")
    chequear(f'const MAINNET_START: i64 = {MAINNET_START};' in lib,
             "MAINNET_START is the instant of the launch",
             f"MAINNET_START is not {MAINNET_START}: the program would wait for another time")
    chequear(re.search(r'pub const NET_CHAIN: &str = "brisvia";', lib) is not None,
             "the real network is named brisvia",
             "cannot find NET_CHAIN=brisvia in the mainnet config")
    chequear('const POOL_ENABLED: bool = false;' in lib,
             "the pool is off in the code",
             "POOL_ENABLED is not false: the pool could turn on and the interface cannot show whether you get paid")
    chequear(re.search(r'"84h/' + COIN_TYPE + r"h/0h\"|84h/9339h/0h", lib) is not None or "9339" in lib,
             f"coin type {COIN_TYPE} present",
             f"cannot find coin type {COIN_TYPE}: the wallet would derive on another branch")

    # ---- Network identity in the core ----
    if CORE.exists():
        chain = leer(CORE / "src/kernel/chainparams.cpp")
        main_blk = chain[chain.find("CBrisviaMainParams()"):]
        main_blk = main_blk[: main_blk.find("class C", 10) if main_blk.find("class C", 10) > 0 else 22000]
        chequear(GENESIS in main_blk, "genesis of the real network correct",
                 f"the mainnet genesis is NOT {GENESIS[:16]}...: it would be another chain")
        chequear(f"genesisTime = {GENESIS_TIME}" in main_blk,
                 "the genesis is dated to the instant of the launch",
                 f"genesisTime is not {GENESIS_TIME}")
        chequear(f'nDefaultPort = {P2P_PORT};' in main_blk, f"P2P port {P2P_PORT}",
                 f"the mainnet P2P port is not {P2P_PORT}")
        chequear(f'bech32_hrp = "{HRP}"' in main_blk, f"{HRP}1... addresses",
                 f'the address prefix is not "{HRP}"')
        chequear("vSeeds.clear()" in main_blk,
                 "no DNS seeds (documented: bootstrap depends on the fixed ones)",
                 "vSeeds changed: review it, bootstrap depends on this")

        # The fixed seeds, decoded from the array (not from the comment).
        seeds_h = leer(CORE / "src/chainparamsseeds.h")
        m = re.search(r"chainparams_seed_brisvia_main\[\]\s*=\s*\{(.*?)\};", seeds_h, re.S)
        if not m:
            fallos.append("cannot find the mainnet fixed-seeds array")
        else:
            b = [int(x, 16) for x in re.findall(r"0x([0-9a-fA-F]{2})", m.group(1))]
            got = []
            for i in range(0, len(b) - 7, 8):
                e = b[i:i + 8]
                if e[0] == 0x01 and e[1] == 0x04:
                    got.append(f"{e[2]}.{e[3]}.{e[4]}.{e[5]}:{(e[6] << 8) | e[7]}")
            for s in SEEDS:
                chequear(s in got, f"seed {s} compiled in",
                         f"MISSING seed {s}: no freshly installed program could find that node")
            chequear(len(got) == len(SEEDS), f"exactly {len(SEEDS)} seeds",
                     f"there are {len(got)} seeds and there should be {len(SEEDS)}: {got}")
    else:
        fallos.append(f"cannot find the core at {CORE}: cannot verify genesis or seeds")

    # ---- Updater key: that it does not change by accident ----
    pub = re.search(r'"pubkey"\s*:\s*"([^"]+)"', tauri)
    chequear(pub is not None and len(pub.group(1)) > 40,
             "updater public key present",
             "cannot find the updater public key: updates would not be verified")

    print(f"RELEASE IDENTITY — {len(oks)} OK, {len(fallos)} failures\n")
    for o in oks:
        print(f"  OK    {o}")
    if fallos:
        print()
        for f in fallos:
            print(f"  FAIL  {f}")
        print("\nDO NOT FREEZE OR COMPILE UNTIL THIS IS RESOLVED.")
        return 1
    print("\nOK: the real network identity is complete and coherent.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
