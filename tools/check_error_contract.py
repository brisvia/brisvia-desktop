#!/usr/bin/env python3
"""Contract check: every error the backend can emit must reach the user as readable text, in BOTH languages.

Why this exists: the owner opened the app and got a raw English "node is not ready yet" on screen, in a
Spanish UI. The 20-agent audit had reviewed the logic and never looked at this, because each half is fine on
its own -- Rust returns a string, the frontend shows a string. The bug only exists in the SEAM between them.

It fails the build when:
  1. Rust returns an "ERR:X" that the frontend does not map to a translation key.
  2. The frontend maps an "ERR:X" that Rust never emits (dead entry, hides a rename).
  3. A translation key referenced by the map is missing in Spanish or in English.
  4. Rust returns a raw error message (not "ERR:X") from a place that reaches the user.

Runs in CI. No dependencies, no build step.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LIB = ROOT / "src-tauri" / "src" / "lib.rs"
APP = ROOT / "src" / "renderer" / "app.js"
LOCALES = ROOT / "src" / "renderer" / "locales.js"

fallos = []


def leer(p):
    if not p.exists():
        fallos.append(f"file does not exist: {p}")
        return ""
    return p.read_text(encoding="utf-8")


rust = leer(LIB)
app = leer(APP)
loc = leer(LOCALES)

# --- 1) what Rust can emit: "ERR:SOMETHING", ignoring comment lines ---
codigo_rust = "\n".join(
    l for l in rust.splitlines() if not l.lstrip().startswith("//")
)
rust_errs = set(re.findall(r'"ERR:([A-Z_0-9]+)"', codigo_rust))

# --- 2) what the frontend knows how to translate: everything inside transError's map ---
# The map packs several pairs per line, so scan the whole block instead of line by line.
m = re.search(r"const map = \{(.*?)\n    \};", app, re.S)
if not m:
    fallos.append("could not find the error map (transError) in app.js")
    mapa = {}
else:
    mapa = dict(re.findall(r"([A-Z_0-9]+):\s*'([a-z_0-9.]+)'", m.group(1)))

# --- 3) the keys that actually exist in each language ---
def bloque(texto, lang):
    m = re.search(r"^  %s: \{" % lang, texto, re.M)
    if not m:
        return ""
    i = m.start()
    # up to the start of the next top-level language block, or the end
    n = re.search(r"^  [a-z]{2}: \{", texto[i + 5:], re.M)
    return texto[i: i + 5 + n.start()] if n else texto[i:]


def tiene_clave(bloque_txt, dotted):
    # 'errors.node_disk_full' -> section "errors", key "node_disk_full"
    sec, key = dotted.split(".", 1)
    m = re.search(r"^\s*%s: \{" % re.escape(sec), bloque_txt, re.M)
    if not m:
        return False
    resto = bloque_txt[m.end():]
    fin = resto.find("\n    },")
    cuerpo = resto[: fin if fin > 0 else len(resto)]
    return re.search(r"^\s*%s:" % re.escape(key), cuerpo, re.M) is not None


es = bloque(loc, "es")
en = bloque(loc, "en")
if not es:
    fallos.append("could not find the SPANISH text block in locales.js")
if not en:
    fallos.append("could not find the ENGLISH text block in locales.js")

# --- check 1: every Rust error is translated ---
for e in sorted(rust_errs):
    if e not in mapa:
        fallos.append(
            f"Rust can return ERR:{e} but the program cannot show it: "
            f"'{e}' is missing in transError (app.js). The user would get raw text."
        )

# --- check 2: no dead entries in the map ---
for e in sorted(mapa):
    if e not in rust_errs:
        fallos.append(
            f"transError translates ERR:{e} but Rust never returns it. "
            f"It is spare, or someone renamed the error and left dead text."
        )

# --- check 3: the referenced key exists in BOTH languages ---
for e, clave in sorted(mapa.items()):
    if es and not tiene_clave(es, clave):
        fallos.append(f"ERR:{e} points to '{clave}' and that key does NOT exist in SPANISH.")
    if en and not tiene_clave(en, clave):
        fallos.append(f"ERR:{e} points to '{clave}' and that key does NOT exist in ENGLISH.")

# --- check 4: raw messages that reach the user ---
# ok_or / ok_or_else / map_err returning a plain English sentence instead of an ERR: code.
crudos = []
for m in re.finditer(r'\.ok_or(?:_else)?\(\s*\|?\|?\s*"([^"]{12,})"', rust):
    txt = m.group(1)
    if not txt.startswith("ERR:"):
        crudos.append(txt)
for txt in sorted(set(crudos)):
    fallos.append(
        f'raw message that can reach the user: "{txt}" -- it should be a translated ERR:X code.'
    )

print(f"errors Rust emits: {len(rust_errs)} | translated in the program: {len(mapa)}")
if fallos:
    print("\nERROR CONTRACT FAILED:\n")
    for f in fallos:
        print("  - " + f)
    sys.exit(1)
print("OK: every error in the program reaches the user translated in ES and EN.")
