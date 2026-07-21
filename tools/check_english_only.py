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

# The offending lines are Spanish, so by definition they carry accented characters. On a Windows console
# the default encoding cannot represent them, and printing the report crashed with UnicodeEncodeError --
# a guard that dies while describing what it found reports nothing at all, and looks like a tooling
# error rather than a finding.
sys.stdout.reconfigure(encoding="utf-8", errors="replace")
sys.stderr.reconfigure(encoding="utf-8", errors="replace")

# Spanish function words: the small connectives that carry no meaning on their own, appear constantly
# in Spanish prose, and are rare or impossible in English technical writing.
#
# Deliberately NOT here, and each for a reason learned by getting it wrong:
#   - "de", "en", "no", "a": collide with English, or with code.
#   - "version", "error", "firma", "prueba", "guardian": these are English words too. With a threshold
#     of two, a line like `let version = tmpl["version"]` scored two hits and got flagged. A checker
#     that rejects correct code is worse than none: it teaches people to switch it off.
PALABRAS = r"\b(que|para|porque|cuando|desde|hasta|donde|aunque|entonces|tambien|todavia|siempre|nunca|" \
           r"puede|tiene|hace|esta|estan|este|esto|esa|ese|eso|los|las|del|una|uno|con|sin|por|" \
           r"sobre|entre|antes|despues|mismo|misma|cada|otro|otra|todo|toda|nada|algo|" \
           r"nodo|billetera|actualizador|actualizacion|candado|verificador|maquina|archivo|" \
           r"arbol|compuerta|borrador|paquete|fallo|usuario|carpeta|" \
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
# Spanish is not banned from the repository -- it is banned from the technical layer. These places hold
# Spanish ON PURPOSE, and a checker that broke them would be breaking the product to satisfy a rule:
EXENTOS = (
    "src/renderer/locales.js",          # the user-facing Spanish. That IS the file's purpose.
    "tools/check_english_only.py",      # this file has to name the words it looks for
    "tools/check_textos.py",            # same: it has to name the Spanish words it hunts in the HTML
)
# Release approvals quote the owner word for word, in the language he wrote them in. Translating a
# quotation would make the record say something he did not say, which defeats the point of keeping it:
# it exists so anyone can see who authorised a release and in what terms.
EXENTOS_PREFIJO = ("owner-approval-v",)
# Same idea, by path fragment: i18n bundles, and fixtures that assert on Spanish user-facing strings
# (a test for "Billetera bloqueada" has to contain "Billetera bloqueada" -- that is the assertion).
EXENTOS_PARCIALES = ("/i18n/", "/locales/", "locales.js", "/fixtures/es", ".es.json")

# Untranslatable proper nouns that legitimate localization shows in their own form: the language selector
# renders each language IN ITS OWN LANGUAGE ("Español" / "English"). This is NOT a global amnesty for the
# word -- it is stripped before scanning ONLY in the files where that word legitimately belongs: the
# selector itself and the doc that describes it. Anywhere else, "Español" is Spanish like any other word.
PERMITIDO_INLINE = ("Español",)
PERMITIDO_INLINE_ARCHIVOS = ("src/renderer/index.html", "PRODUCT_CONTRACTS.md")

# Marker for locale (es) text that must live in code: the native tray menu/tooltip are built in Rust, not
# in locales.js, so they localize with `if lang == "es" { ... }`. It is NOT a free "ignore this line".
# It is honored ONLY when ALL of these hold (enforced in marca_valida, proven by the self-test):
#   - the file is in the authorized list below (the marker anywhere else FAILS);
#   - the line carries a real string literal (the thing being localized);
#   - the line actually contains Spanish (a marker with no Spanish is a stray leftover and FAILS);
#   - there is NO Spanish OUTSIDE that literal (a Spanish comment next to it still FAILS).
MARCA_LOCALE = "i18n-es"
MARCA_LOCALE_ARCHIVOS = ("src-tauri/src/lib.rs",)

# String literals ("..." / '...'), to tell Spanish INSIDE a localized literal from Spanish loose in code
# or a comment on the same line.
_LITERAL = re.compile(r'"(?:[^"\\]|\\.)*"' + r"|'(?:[^'\\]|\\.)*'")


def tracked():
    r = subprocess.run(["git", "ls-files"], capture_output=True, text=True)
    return [Path(l) for l in r.stdout.splitlines() if l]


def _es_score(texto: str):
    """(function-word hits, Spanish-only characters) for one line."""
    hits = len(re.findall(PALABRAS, texto.replace("_", " "), re.IGNORECASE))
    raros = SOLO_ESPANOL.findall(texto)
    return hits, raros


def marca_valida(ruta: str, linea: str):
    """A line carries MARCA_LOCALE. Return None if it is a legitimate locale literal, else why it is not."""
    if ruta not in MARCA_LOCALE_ARCHIVOS:
        return "i18n-es marker used outside the authorized files"
    if not _LITERAL.search(linea):
        return "i18n-es marker without a string literal to localize"
    if not any(_es_score(linea)):
        return "i18n-es marker on a line with no Spanish (stray marker)"
    # Spanish must live INSIDE the literal only. Blank out the literals; any Spanish left over was loose in
    # code or a comment. One hit is enough here -- a marked line must carry nothing but the localized text.
    resto = _LITERAL.sub('""', linea)
    r_hits, r_raros = _es_score(resto)
    if r_hits >= 1 or r_raros:
        return "Spanish outside the localized literal on an i18n-es line"
    return None


def revisar(p: Path) -> list:
    ruta = str(p).replace("\\", "/")
    if (ruta in EXENTOS or any(f in ruta for f in EXENTOS_PARCIALES)
            or ruta.startswith(EXENTOS_PREFIJO) or p.suffix not in EXTENSIONES):
        return []
    try:
        texto = p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return []
    malas = []
    for n, linea in enumerate(texto.splitlines(), 1):
        if MARCA_LOCALE in linea:
            motivo = marca_valida(ruta, linea)
            if motivo is None:
                continue                     # a legitimate locale literal, tightly scoped
            malas.append((n, linea.strip()[:96], 0, motivo))
            continue
        if len(linea.strip()) < 12:
            continue
        # "Español" is whitelisted ONLY in the files where the language selector legitimately shows it.
        scan = linea
        if ruta in PERMITIDO_INLINE_ARCHIVOS:
            for w in PERMITIDO_INLINE:
                scan = scan.replace(w, "")
        # Underscores have to break words, or identifiers slip through. `` does not split on `_`
        # (it is a word character), so `si_el_nodo_no_cierra` reads as one unknown token and passes.
        # That is not hypothetical: it is exactly how nine Spanish test names reached a public tag,
        # and the self-test caught it on the first run of this checker.
        hits, raros = _es_score(scan)
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
        # Locale text in code is caught. The i18n-es marker does NOT rescue it in an unauthorized file
        # (this temp .py is not the authorized lib.rs), and "Español" is not whitelisted here either.
        ('        format!("Brisvia — Minando al {}% de {} núcleos", p, c)', True),
        ('        format!("Brisvia — Minando al {}% de {} núcleos", p, c) // i18n-es', True),
        ('    <button class="seg-btn" data-lang="es">Español</button>', True),
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

    # The marker's guard rails, checked directly (revisar keys on the real path, so a temp file cannot
    # stand in for the authorized route). True = accepted as locale, False = rejected with a reason.
    LIB = MARCA_LOCALE_ARCHIVOS[0]
    marca_casos = [
        (LIB, '  format!("Brisvia — Minando al {}% de {} núcleos", p, c) // i18n-es (tray tooltip)', True),
        (LIB, '  if lang == "es" { ("Abrir Brisvia", "Salir de Brisvia") } else { ("Open", "Exit") } // i18n-es', True),
        (LIB, '  let n = threads; // i18n-es', False),                              # stray: no Spanish
        (LIB, '  let x = "Abrir Brisvia"; // no lo mates nunca i18n-es', False),    # Spanish loose in a comment
        ("tools/foo.py", '  s = "Abrir Brisvia" # i18n-es', False),                # unauthorized file
    ]
    for ruta, linea, esperado_ok in marca_casos:
        acepta = marca_valida(ruta, linea) is None
        bien = acepta == esperado_ok
        ok &= bien
        print(f"  {'OK ' if bien else 'BAD'}  marker {'accepts' if esperado_ok else 'rejects':<8} {linea[:52]}")

    # "Español" is stripped (allowed) only in its authorized files; kept (caught) anywhere else.
    for ruta, esperado_pego in (("src/renderer/index.html", False), ("tools/other.py", True)):
        scan = '<button data-lang="es">Español</button>'
        if ruta in PERMITIDO_INLINE_ARCHIVOS:
            for w in PERMITIDO_INLINE:
                scan = scan.replace(w, "")
        pego = any(_es_score(scan))
        bien = pego == esperado_pego
        ok &= bien
        print(f"  {'OK ' if bien else 'BAD'}  Español@{ruta.split('/')[-1]:<18} {'caught' if pego else 'allowed'}")

    print("\n" + ("OK: it catches Spanish and allows only scoped localization." if ok
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
    print("OK: no Spanish outside approved localization scope.")
