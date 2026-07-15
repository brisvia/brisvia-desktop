# -*- coding: utf-8 -*-
"""Guarda de identidad de la release: todo lo que define "esto es la red real de Brisvia".

Por que existe: cada uno de estos valores, si sale mal, produce un fallo SILENCIOSO que solo se descubre
el dia del lanzamiento, cuando ya hay gente con el programa instalado. Una red equivocada, una billetera
derivada en otra rama, una pool que se enciende sola, o el minado habilitado antes de tiempo no dan error:
simplemente hacen otra cosa. Este script los pone todos en un solo lugar y falla si alguno se movio.

Corre SIN compilar: lee las fuentes. La verificacion sobre el binario ya construido es aparte
(los workflows extraen el sidecar y lo ejecutan).

Uso:  python tools/check_release_identity.py [--version 1.0.6]
"""
import argparse
import pathlib
import re
import sys

RAIZ = pathlib.Path(__file__).resolve().parents[1]
CORE = RAIZ.parent / "cripto-pow"

# Los valores canonicos de la red real. Si alguno cambia a proposito, se cambia ACA, en el mismo commit,
# y se explica por que. Que este script se ponga verde solo no significa que el cambio sea correcto.
GENESIS = "aa6bc268339aa9f4f2e39ae33aca7b7e48e395033d08d37c08f828890af7baf7"
GENESIS_TIME = "1785596400"   # 1-ago-2026 15:00 UTC: el instante del lanzamiento
MAINNET_START = "1_785_596_400"
COIN_TYPE = "9339"
HRP = "brv"
P2P_PORT = "9333"
SEEDS = ["187.77.240.145:9333", "129.80.250.36:9333", "129.159.108.102:9333"]


def leer(p: pathlib.Path) -> str:
    return p.read_text(encoding="utf-8", errors="replace")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", help="version esperada (ej: 1.0.6). Si se omite, solo chequea que coincidan entre si.")
    args = ap.parse_args()

    fallos, oks = [], []

    def chequear(cond, ok_msg, fail_msg):
        (oks if cond else fallos).append(ok_msg if cond else fail_msg)

    # ---- Version: las dos unicas fuentes permitidas ----
    cargo = leer(RAIZ / "src-tauri/Cargo.toml")
    tauri = leer(RAIZ / "src-tauri/tauri.conf.json")
    v_cargo = re.search(r'^version = "([0-9.]+)"', cargo, re.M)
    v_tauri = re.search(r'"version": "([0-9.]+)"', tauri)
    chequear(v_cargo and v_tauri, "version leible en las dos fuentes", "no pude leer la version")
    if v_cargo and v_tauri:
        chequear(v_cargo.group(1) == v_tauri.group(1),
                 f"version sincronizada ({v_cargo.group(1)})",
                 f"VERSION DESINCRONIZADA: Cargo.toml={v_cargo.group(1)} vs tauri.conf.json={v_tauri.group(1)} "
                 f"-> el actualizador ofreceria una version distinta a la que trae el instalador")
        if args.version:
            chequear(v_cargo.group(1) == args.version,
                     f"version es la esperada ({args.version})",
                     f"la version es {v_cargo.group(1)} y se esperaba {args.version}")

    # ---- Identidad de red en el minero ----
    lib = leer(RAIZ / "src-tauri/src/lib.rs")
    chequear(f'const MAINNET_START: i64 = {MAINNET_START};' in lib,
             "MAINNET_START es el instante del lanzamiento",
             f"MAINNET_START no es {MAINNET_START}: el programa esperaria a otra hora")
    chequear(re.search(r'pub const NET_CHAIN: &str = "brisvia";', lib) is not None,
             "la red real se llama brisvia",
             "no encuentro NET_CHAIN=brisvia en la config de mainnet")
    chequear('const POOL_ENABLED: bool = false;' in lib,
             "la pool esta apagada en el codigo",
             "POOL_ENABLED no es false: la pool podria encenderse y la interfaz no sabe mostrar si te pagan")
    chequear(re.search(r'"84h/' + COIN_TYPE + r"h/0h\"|84h/9339h/0h", lib) is not None or "9339" in lib,
             f"coin type {COIN_TYPE} presente",
             f"no encuentro el coin type {COIN_TYPE}: la billetera derivaria en otra rama")

    # ---- Identidad de red en el nucleo ----
    if CORE.exists():
        chain = leer(CORE / "src/kernel/chainparams.cpp")
        main_blk = chain[chain.find("CBrisviaMainParams()"):]
        main_blk = main_blk[: main_blk.find("class C", 10) if main_blk.find("class C", 10) > 0 else 22000]
        chequear(GENESIS in main_blk, "genesis de la red real correcto",
                 f"el genesis de mainnet NO es {GENESIS[:16]}...: seria otra cadena")
        chequear(f"genesisTime = {GENESIS_TIME}" in main_blk,
                 "el genesis esta fechado en el instante del lanzamiento",
                 f"genesisTime no es {GENESIS_TIME}")
        chequear(f'nDefaultPort = {P2P_PORT};' in main_blk, f"puerto P2P {P2P_PORT}",
                 f"el puerto P2P de mainnet no es {P2P_PORT}")
        chequear(f'bech32_hrp = "{HRP}"' in main_blk, f"direcciones {HRP}1...",
                 f'el prefijo de direcciones no es "{HRP}"')
        chequear("vSeeds.clear()" in main_blk,
                 "sin semillas DNS (documentado: el arranque depende de las fijas)",
                 "vSeeds cambio: revisar, el arranque depende de esto")

        # Las semillas fijas, decodificadas del array (no del comentario).
        seeds_h = leer(CORE / "src/chainparamsseeds.h")
        m = re.search(r"chainparams_seed_brisvia_main\[\]\s*=\s*\{(.*?)\};", seeds_h, re.S)
        if not m:
            fallos.append("no encuentro el array de semillas fijas de mainnet")
        else:
            b = [int(x, 16) for x in re.findall(r"0x([0-9a-fA-F]{2})", m.group(1))]
            got = []
            for i in range(0, len(b) - 7, 8):
                e = b[i:i + 8]
                if e[0] == 0x01 and e[1] == 0x04:
                    got.append(f"{e[2]}.{e[3]}.{e[4]}.{e[5]}:{(e[6] << 8) | e[7]}")
            for s in SEEDS:
                chequear(s in got, f"semilla {s} compilada",
                         f"FALTA la semilla {s}: ningun programa recien instalado podria encontrar ese nodo")
            chequear(len(got) == len(SEEDS), f"exactamente {len(SEEDS)} semillas",
                     f"hay {len(got)} semillas y deberian ser {len(SEEDS)}: {got}")
    else:
        fallos.append(f"no encuentro el nucleo en {CORE}: no puedo verificar genesis ni semillas")

    # ---- Clave del actualizador: que no cambie por accidente ----
    pub = re.search(r'"pubkey"\s*:\s*"([^"]+)"', tauri)
    chequear(pub is not None and len(pub.group(1)) > 40,
             "clave publica del actualizador presente",
             "no encuentro la clave publica del actualizador: las actualizaciones no se verificarian")

    print(f"IDENTIDAD DE LA RELEASE — {len(oks)} OK, {len(fallos)} fallos\n")
    for o in oks:
        print(f"  OK    {o}")
    if fallos:
        print()
        for f in fallos:
            print(f"  FALLA {f}")
        print("\nNO CONGELAR NI COMPILAR HASTA RESOLVER ESTO.")
        return 1
    print("\nOK: la identidad de la red real esta completa y coherente.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
