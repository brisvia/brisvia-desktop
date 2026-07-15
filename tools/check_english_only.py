#!/usr/bin/env python3
"""Fail if Spanish reaches the public repository.

WHY THIS FILE EXISTS
--------------------
Brisvia is open source. Anyone can read this repository, and most of them do not speak Spanish. So
everything that ships -- comments, log lines, identifiers, docs, workflow step names -- is written in
English from the start. Communication with the owner stays in Spanish; the repository does not.

That rule already existed, written down, and it was broken anyway: roughly 300 lines of Spanish across
eight files landed in one afternoon, including nine test names that reached a public tag.

Which is the same lesson this project already paid for once: a rule that lives in a document stops
nothing. `build-linux.yml` said, in writing, "No -DARCH=native on purpose" -- and build-macos.yml used
-DARCH=native anyway, in the same repo, for months. If something must not happen, a step has to FAIL
when it happens.

HOW IT DECIDES
--------------
Not by dictionary lookup: too many words are spelled the same in both languages, and a false positive
that blocks a build teaches people to disable the check.

It looks for Spanish function words -- the small connectives that carry no meaning on their own and
appear constantly in Spanish prose while being rare or impossible in English: "que", "para", "porque",
"cuando", "esta", "los", "las", "del". Plus characters that only Spanish uses here: ñ, ¿, ¡, and accented
vowels.

One hit is noise. Several on the same line is Spanish prose.

WHAT IT SKIPS, AND WHY
----------------------
  - This file, and anything under tools/ that exists to talk to the owner rather than to ship.
  - Locale files: they hold the user-facing Spanish on purpose, that IS their job.
  - Anything not tracked by git: if it does not ship, it does not matter.

Usage:
    python tools/check_english_only.py                 check everything tracked
    python tools/check_english_only.py <file>...        check specific files
    python tools/check_english_only.py --self-test      prove it still catches Spanish
"""
import re
import subprocess
import sys
from pathlib import Path

# Spanish function words. Chosen because they are frequent in Spanish and rare or absent in English
# technical prose. "de", "en", "no", "a" are deliberately absent: they collide with English or with
# code.
PALABRAS = r"\b(que|para|porque|cuando|desde|hasta|donde|aunque|entonces|tambien|todavia|siempre|nunca|" \
           r"puede|tiene|hace|esta|estan|este|esto|esa|ese|eso|los|las|del|una|uno|con|sin|por|" \
           r"sobre|entre|antes|despues|mismo|misma|cada|otro|otra|todo|toda|nada|algo|" \
           r"nodo|billetera|actualizador|actualizacion|guardian|candado|verificador|maquina|archivo|" \
           r"arbol|compuerta|borrador|firma|paquete|prueba|fallo|error|usuario|version|carpeta|" \
           r"cierra|cerrar|devuelve|espera|esperar|mata|matar|falla|fallar|abre|abrir|corre|correr|" \
           r"lanza|lanzar|guarda|guardar|revisa|revisar|probar|busca|buscar|arma|armar|sigue|seguir|" \
           r"queda|quedar|pide|pedir|dice|decir|sale|salir|entra|entrar|vuelve|volver|tarda|tardar)\b"

# Characters that in this repo only appear in Spanish. Accented vowels are included because English
# technical prose here does not use them.
SOLO_ESPANOL = re.compile(r"[ñÑ¿¡áéíóúÁÉÍÓÚ]")

# Verbs are in the list above for a reason: without them, an identifier like
# `si_el_nodo_no_cierra_devuelve_false` hits only one word ("nodo") and slips under the threshold --
# which is exactly the shape that reached a public tag. The self-test caught that on the first run.

# Two hits on one line is prose, not a coincidence. One is not enough: "para" appears in "parameter"
# boundaries, "esta" in data, "version" and "error" are English words too -- that is why the bar is two.
UMBRAL = 2

EXTENSIONES = {".rs", ".py", ".js", ".ts", ".yml", ".yaml", ".md", ".nsh", ".ps1", ".sh", ".toml",
               ".json", ".html", ".css"}

# Files whose job is to hold Spanish, or to talk to the owner rather than ship.
EXENTOS = (
    "src/renderer/locales.js",          # the user-facing Spanish. That IS the file's purpose.
    "tools/check_english_only.py",      # this file names the words it looks for
)


def tracked():
    r = subprocess.run(["git", "ls-files"], capture_output=True, text=True)
    return [Path(l) for l in r.stdout.splitlines() if l]


def revisar(p: Path) -> list:
    if str(p).replace("\\", "/") in EXENTOS or p.suffix not in EXTENSIONES:
        return []
    try:
        texto = p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return []
    malas = []
    for n, linea in enumerate(texto.splitlines(), 1):
        if len(linea.strip()) < 12:
            continue
        # Underscores have to break words, or identifiers slip through. `` does not split on `_`
        # (it is a word character), so `si_el_nodo_no_cierra` reads as one unknown token and passes.
        # That is not hypothetical: it is exactly how nine Spanish test names reached a public tag,
        # and the self-test caught it on the first run of this checker.
        hits = len(re.findall(PALABRAS, linea.replace("_", " "), re.IGNORECASE))
        raros = SOLO_ESPANOL.findall(linea)
        if hits >= UMBRAL or raros:
            malas.append((n, linea.strip()[:96], hits, "".join(sorted(set(raros)))))
    return malas


def self_test() -> int:
    """A checker that cannot catch the thing it exists to catch is decoration."""
    import tempfile
    print("=== self-test: does it still catch Spanish? ===")
    casos = [
        ("# el nodo tiene que cerrar antes de que el instalador toque un archivo", True),
        ("// Wait for the node to exit before the installer touches a single file", False),
        ("    # verificar que el paquete no este mezclado con otro candidato", True),
        ("    let max = Duration::from_secs(180); // ceiling, not a wait", False),
        ("fn si_el_nodo_no_cierra_devuelve_false() {", True),
        ("fn node_still_running_returns_false() {", False),
        ("# la señal nunca se termina", True),
        ("SHA-256 of the downloaded package, not of the local copy", False),
    ]
    ok = True
    d = Path(tempfile.mkdtemp())
    for i, (linea, deberia) in enumerate(casos):
        f = d / f"c{i}.py"
        f.write_text(linea, encoding="utf-8")
        pego = bool(revisar(f))
        bien = pego == deberia
        ok &= bien
        print(f"  {'OK ' if bien else 'BAD'}  {'catches' if deberia else 'allows ':<8}  {linea[:62]}")
    print("\n" + ("OK: it catches Spanish and lets English through." if ok
                  else "BAD: the checker is wrong. Fix it before trusting it."))
    return 0 if ok else 1


if __name__ == "__main__":
    args = sys.argv[1:]
    if args and args[0] == "--self-test":
        sys.exit(self_test())
    archivos = [Path(a) for a in args] if args else tracked()
    total = 0
    for p in sorted(archivos):
        if (m := revisar(p)):
            print(f"\n{p}  ({len(m)} lines)")
            for n, l, h, r in m[:6]:
                print(f"  {n:>5}: {l}")
            if len(m) > 6:
                print(f"        ... and {len(m) - 6} more")
            total += len(m)
    if total:
        print(f"\nREJECTED: {total} lines of Spanish in a public repository.")
        print("Brisvia is open source. Everything that ships is written in English: comments, logs,")
        print("identifiers, docs, workflow step names. Spanish is for talking to the owner, not for")
        print("the repo. Translate them, do not silence this check.")
        sys.exit(1)
    print("OK: no Spanish in what ships.")
