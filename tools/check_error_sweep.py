# -*- coding: utf-8 -*-
"""Error sweep: no backend ERR: may reach the user without text in their language.

Why it exists: the frontend (app.js transError) shows ANY string it does not recognize AS-IS. If the
backend invents an ERR:NEW and nobody maps it, the user sees "ERR:NEW" on screen. And if friendly_error
lets the node's raw text through, the user reads internal English and their own personal folder on a
screen that moves money. This script fails if that can happen.

Usage:  python tools/check_error_sweep.py
"""
import pathlib
import re
import sys

RAIZ = pathlib.Path(__file__).resolve().parents[1]


def main() -> int:
    rust = (RAIZ / "src-tauri/src/lib.rs").read_text(encoding="utf-8")
    app = (RAIZ / "src/renderer/app.js").read_text(encoding="utf-8")
    loc = (RAIZ / "src/renderer/locales.js").read_text(encoding="utf-8")

    fallos = []

    # 1) Every ERR: the backend produces must be in the frontend map.
    #    "ERR:CODE" appears in the comment that explains the format: it is not a real code.
    codigos = {c[4:] for c in re.findall(r'"(ERR:[A-Z_]+)"', rust)} - {"CODE"}
    # The map packs several per line; without a line-start anchor (a ^ here would see only the first).
    mapeados = set(re.findall(r"([A-Z_]{3,}):\s*'errors\.", app))
    for falta in sorted(codigos - mapeados):
        fallos.append(f"ERR:{falta} is produced by the backend and is NOT in the map: the user would see the raw code")

    # 2) Every key in the map must have text, and in both languages.
    claves = set(re.findall(r"'errors\.([a-z_]+)'", app))
    for k in sorted(claves):
        n = len(re.findall(rf"\b{k}:\s*'", loc))
        if n == 0:
            fallos.append(f"errors.{k} is mapped but has NO text in locales.js")
        elif n < 2:
            fallos.append(f"errors.{k} has text in {n} language(s): translations missing")

    # 3) friendly_error must not return the node's raw message.
    cuerpo = re.search(r"fn friendly_error\(.*?\n\}", rust, re.S)
    if cuerpo and re.search(r"^\s*msg\.to_string\(\)\s*$", cuerpo.group(0), re.M):
        fallos.append(
            "friendly_error returns msg.to_string() as a fallback: the node's raw text (with personal "
            "paths and the word 'Bitcoin') would reach the user. It must return a sanitized code."
        )

    print(f"backend codes: {len(codigos)} | mapped: {len(mapeados)} | keys with text: {len(claves)}")
    if fallos:
        print("\nERROR SWEEP FAILED:\n")
        for f in fallos:
            print(f"  - {f}")
        return 1
    print("OK: every backend error arrives translated, in both languages, and none raw.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
