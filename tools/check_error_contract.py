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

failures = []


def read(p):
    if not p.exists():
        failures.append(f"file does not exist: {p}")
        return ""
    return p.read_text(encoding="utf-8")


rust = read(LIB)
app = read(APP)
loc = read(LOCALES)

# --- 1) what Rust can emit: "ERR:SOMETHING", ignoring comment lines ---
rust_code = "\n".join(
    l for l in rust.splitlines() if not l.lstrip().startswith("//")
)
rust_errs = set(re.findall(r'"ERR:([A-Z_0-9]+)"', rust_code))

# --- 2) what the frontend knows how to translate: everything inside transError's map ---
# The map packs several pairs per line, so scan the whole block instead of line by line.
m = re.search(r"const map = \{(.*?)\n    \};", app, re.S)
if not m:
    failures.append("could not find the error map (transError) in app.js")
    err_map = {}
else:
    err_map = dict(re.findall(r"([A-Z_0-9]+):\s*'([a-z_0-9.]+)'", m.group(1)))

# --- 3) the keys that actually exist in each language ---
def lang_block(text, lang):
    m = re.search(r"^  %s: \{" % lang, text, re.M)
    if not m:
        return ""
    i = m.start()
    # up to the start of the next top-level language block, or the end
    n = re.search(r"^  [a-z]{2}: \{", text[i + 5:], re.M)
    return text[i: i + 5 + n.start()] if n else text[i:]


def has_key(block_txt, dotted):
    # 'errors.node_disk_full' -> section "errors", key "node_disk_full"
    sec, key = dotted.split(".", 1)
    m = re.search(r"^\s*%s: \{" % re.escape(sec), block_txt, re.M)
    if not m:
        return False
    rest = block_txt[m.end():]
    end = rest.find("\n    },")
    body = rest[: end if end > 0 else len(rest)]
    return re.search(r"^\s*%s:" % re.escape(key), body, re.M) is not None


es = lang_block(loc, "es")
en = lang_block(loc, "en")
if not es:
    failures.append("could not find the SPANISH text block in locales.js")
if not en:
    failures.append("could not find the ENGLISH text block in locales.js")

# --- check 1: every Rust error is translated ---
for e in sorted(rust_errs):
    if e not in err_map:
        failures.append(
            f"Rust can return ERR:{e} but the program cannot show it: "
            f"'{e}' is missing in transError (app.js). The user would get raw text."
        )

# --- check 2: no dead entries in the map ---
for e in sorted(err_map):
    if e not in rust_errs:
        failures.append(
            f"transError translates ERR:{e} but Rust never returns it. "
            f"It is spare, or someone renamed the error and left dead text."
        )

# --- check 3: the referenced key exists in BOTH languages ---
for e, key in sorted(err_map.items()):
    if es and not has_key(es, key):
        failures.append(f"ERR:{e} points to '{key}' and that key does NOT exist in SPANISH.")
    if en and not has_key(en, key):
        failures.append(f"ERR:{e} points to '{key}' and that key does NOT exist in ENGLISH.")

# --- check 4: raw messages that reach the user ---
# ok_or / ok_or_else / map_err returning a plain English sentence instead of an ERR: code.
raw_msgs = []
for m in re.finditer(r'\.ok_or(?:_else)?\(\s*\|?\|?\s*"([^"]{12,})"', rust):
    txt = m.group(1)
    if not txt.startswith("ERR:"):
        raw_msgs.append(txt)
for txt in sorted(set(raw_msgs)):
    failures.append(
        f'raw message that can reach the user: "{txt}" -- it should be a translated ERR:X code.'
    )

print(f"errors Rust emits: {len(rust_errs)} | translated in the program: {len(err_map)}")
if failures:
    print("\nERROR CONTRACT FAILED:\n")
    for f in failures:
        print("  - " + f)
    sys.exit(1)
print("OK: every error in the program reaches the user translated in ES and EN.")
