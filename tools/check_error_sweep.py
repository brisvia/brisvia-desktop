# -*- coding: utf-8 -*-
"""Barrido de errores: ningun ERR: del backend puede llegar al usuario sin texto en su idioma.

Por que existe: el frontend (app.js transError) muestra TAL CUAL cualquier string que no reconoce. Si el
backend inventa un ERR:NUEVO y nadie lo mapea, el usuario ve "ERR:NUEVO" en pantalla. Y si friendly_error
deja pasar el texto crudo del nodo, el usuario lee ingles interno y su propia carpeta personal en una
pantalla que mueve plata. Este script falla si eso puede pasar.

Uso:  python tools/check_error_sweep.py
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

    # 1) Todo ERR: que el backend produce tiene que estar en el mapa del frontend.
    #    "ERR:CODE" aparece en el comentario que explica el formato: no es un codigo real.
    codigos = {c[4:] for c in re.findall(r'"(ERR:[A-Z_]+)"', rust)} - {"CODE"}
    # El mapa pone varios por linea; sin ancla de inicio de linea (un ^ aca solo veria el primero).
    mapeados = set(re.findall(r"([A-Z_]{3,}):\s*'errors\.", app))
    for falta in sorted(codigos - mapeados):
        fallos.append(f"ERR:{falta} lo produce el backend y NO esta en el mapa: el usuario veria el codigo crudo")

    # 2) Toda clave del mapa tiene que tener texto, y en los dos idiomas.
    claves = set(re.findall(r"'errors\.([a-z_]+)'", app))
    for k in sorted(claves):
        n = len(re.findall(rf"\b{k}:\s*'", loc))
        if n == 0:
            fallos.append(f"errors.{k} esta mapeado pero NO tiene texto en locales.js")
        elif n < 2:
            fallos.append(f"errors.{k} tiene texto en {n} idioma(s): faltan traducciones")

    # 3) friendly_error no puede devolver el mensaje crudo del nodo.
    cuerpo = re.search(r"fn friendly_error\(.*?\n\}", rust, re.S)
    if cuerpo and re.search(r"^\s*msg\.to_string\(\)\s*$", cuerpo.group(0), re.M):
        fallos.append(
            "friendly_error devuelve msg.to_string() como fallback: el texto crudo del nodo (con rutas "
            "personales y la palabra 'Bitcoin') llegaria al usuario. Debe devolver un codigo saneado."
        )

    print(f"codigos del backend: {len(codigos)} | mapeados: {len(mapeados)} | claves con texto: {len(claves)}")
    if fallos:
        print("\nFALLA EL BARRIDO DE ERRORES:\n")
        for f in fallos:
            print(f"  - {f}")
        return 1
    print("OK: todos los errores del backend llegan traducidos, en los dos idiomas, y ninguno crudo.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
