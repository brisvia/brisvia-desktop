#!/usr/bin/env python3
"""Text contract: the app must never mix languages or show untranslated text.

Why this exists: the owner opened the app with the language set to English and found a field reading
"host:puerto · ej: pool.ejemplo.com:3333" -- Spanish, hard-coded in the HTML the same day. A 20-agent audit
had reviewed the logic and missed it, because the i18n system only sees elements marked with data-i18n:
anything typed straight into the HTML is invisible to it. This check looks at the product, not the logic.

It fails the build when:
  1. A key exists in one language and not the other (the user would see it raw, or in the wrong language).
  2. Spanish leaks into the English dictionary, or English into the Spanish one.
  3. The HTML has visible text with no data-i18n / data-i18n-attr (it can never be translated).

Anything legitimately untranslatable (the brand, a sample address, the language buttons) is whitelisted
below, on purpose and one by one: the list is short so that adding to it is a deliberate decision.

Runs in CI. No dependencies, no build step.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOC = ROOT / "src" / "renderer" / "locales.js"
HTML = ROOT / "src" / "renderer" / "index.html"

# Text that must NOT be translated. Each entry is a decision, not an oversight:
PERMITIDO = {
    "Brisvia",   # the brand
    "BRVA",      # the ticker
    "Español",   # the language buttons show each language in ITS OWN language, on purpose
    "English",
    "brv1q…",    # a sample address: the same in every language
    "••••••••",  # password dots
    "H/s",       # hashes per second: a unit, like km/h. The same in every language.
}

fallos = []
loc = LOC.read_text(encoding="utf-8")
html = HTML.read_text(encoding="utf-8")

# ---------- the two dictionaries ----------
i_es, i_en = loc.index("es: {"), loc.index("en: {")
blk_es, blk_en = loc[i_es:i_en], loc[i_en:]


def claves(bloque):
    out, sec = {}, None
    for line in bloque.splitlines():
        m = re.match(r"\s{4}(\w+):\s*\{", line)
        if m:
            sec = m.group(1)
            continue
        for k, v in re.findall(r"(\w+):\s*'((?:[^'\\]|\\.)*)'", line):
            if sec:
                out[f"{sec}.{k}"] = v
    return out


kes, ken = claves(blk_es), claves(blk_en)
print(f"ES keys: {len(kes)} | EN keys: {len(ken)}")

# ---------- 1) missing keys ----------
for k in sorted(set(kes) - set(ken)):
    fallos.append(f"key '{k}' exists in SPANISH but is missing in ENGLISH.")
for k in sorted(set(ken) - set(kes)):
    fallos.append(f"key '{k}' exists in ENGLISH but is missing in SPANISH.")

# ---------- 2) mixed languages ----------
for k, v in sorted(ken.items()):
    if re.search(r"[áéíóúñ¿¡]", v, re.I) or re.search(
        r"\b(puedes|billetera|contraseña|puerto|ejemplo|computadora|minado|guardar)\b", v, re.I
    ):
        fallos.append(f"Spanish text inside the ENGLISH dictionary: {k} = \"{v[:70]}\"")
for k, v in sorted(kes.items()):
    if re.search(r"\b(you can|your wallet|the pool|password|mining|please|not ready)\b", v, re.I):
        fallos.append(f"English text inside the SPANISH dictionary: {k} = \"{v[:70]}\"")

# ---------- 3) hard-coded text in the HTML ----------
for m in re.finditer(r'<(input|textarea)[^>]*placeholder="([^"]+)"[^>]*>', html):
    if "data-i18n" not in m.group(0) and m.group(2) not in PERMITIDO:
        fallos.append(
            f'hard-coded HTML text that never gets translated: placeholder "{m.group(2)}" '
            f"-- use data-i18n-attr=\"placeholder:section.key\"."
        )
for m in re.finditer(
    r'<(span|div|p|h[1-4]|button|label)(?![^>]*data-i18n)[^>]*>([^<>{]{2,60})</\1>', html
):
    txt = m.group(2).strip()
    if not txt or re.fullmatch(r"[\W\d\s·—…]+", txt) or txt.startswith("&") or txt in PERMITIDO:
        continue
    fallos.append(f'hard-coded HTML text that never gets translated: <{m.group(1)}>"{txt[:50]}"')

if fallos:
    print("\nTEXT CONTRACT FAILED:\n")
    for f in fallos:
        print("  - " + f)
    sys.exit(1)
print("OK: both languages are complete, no mixing, and the HTML has no hard-coded untranslated text.")
