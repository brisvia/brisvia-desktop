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

ROOT = pathlib.Path(__file__).resolve().parents[1]


def main() -> int:
    rust = (ROOT / "src-tauri/src/lib.rs").read_text(encoding="utf-8")
    app = (ROOT / "src/renderer/app.js").read_text(encoding="utf-8")
    loc = (ROOT / "src/renderer/locales.js").read_text(encoding="utf-8")

    failures = []

    # 1) Every ERR: the backend produces must be in the frontend map.
    #    "ERR:CODE" appears in the comment that explains the format: it is not a real code.
    codes = {c[4:] for c in re.findall(r'"(ERR:[A-Z_]+)"', rust)} - {"CODE"}
    # The map packs several per line; without a line-start anchor (a ^ here would see only the first).
    mapped = set(re.findall(r"([A-Z_]{3,}):\s*'errors\.", app))
    for missing in sorted(codes - mapped):
        failures.append(f"ERR:{missing} is produced by the backend and is NOT in the map: the user would see the raw code")

    # 2) Every key in the map must have text, and in both languages.
    keys = set(re.findall(r"'errors\.([a-z_]+)'", app))
    for k in sorted(keys):
        n = len(re.findall(rf"\b{k}:\s*'", loc))
        if n == 0:
            failures.append(f"errors.{k} is mapped but has NO text in locales.js")
        elif n < 2:
            failures.append(f"errors.{k} has text in {n} language(s): translations missing")

    # 3) friendly_error must not return the node's raw message.
    body = re.search(r"fn friendly_error\(.*?\n\}", rust, re.S)
    if body and re.search(r"^\s*msg\.to_string\(\)\s*$", body.group(0), re.M):
        failures.append(
            "friendly_error returns msg.to_string() as a fallback: the node's raw text (with personal "
            "paths and the word 'Bitcoin') would reach the user. It must return a sanitized code."
        )

    print(f"backend codes: {len(codes)} | mapped: {len(mapped)} | keys with text: {len(keys)}")
    if failures:
        print("\nERROR SWEEP FAILED:\n")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("OK: every backend error arrives translated, in both languages, and none raw.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
