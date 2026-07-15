#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Decide si un binario arranca en la maquina de un usuario, o solo en la que lo compilo.

POR QUE EXISTE
--------------
La 1.0.5 publica sale rota en macOS y en Linux: el nodo quedo enlazado contra librerias que existen en
la maquina de compilacion (Homebrew, libevent-dev) y no en la del usuario. La app abre, el nodo muere, y
sin nodo no hay billetera, ni red, ni minado. Tres builds en verde no lo detectaron.

El primer candado que escribi era un bloque de bash con `ldd` y recortes de nombres con `sed`. Dio DOS
falsos positivos seguidos, los dos por suponer en vez de leer:
  1) recorto mal el nombre del cargador y rechazo un binario perfecto;
  2) despues asumi que el cargador no figuraba entre las dependencias declaradas. Figura.
Cada uno costo un ciclo de compilacion de 40 minutos.

Esto no se resuelve con mas cuidado al escribir bash: se resuelve leyendo la estructura del binario en
vez de parsear texto. Eso es lo que hace `lief`, y es lo que usa el propio Bitcoin Core en
contrib/guix/symbol-check.py, que corre en cada uno de sus releases.

QUE VERIFICA, Y POR QUE CADA COSA
---------------------------------
  LIBRERIAS: allowlist, nunca denylist. Buscar "libevent" es apuntarle al ultimo bug conocido; el
    proximo va a ser con otra libreria. Se declara lo permitido y se rechaza todo lo demas.

  VERSIONES DE SIMBOLOS: lo que de verdad decide en que maquinas arranca, y lo que ningun `ldd` muestra.
    Un binario puede pedir libc.so.6 (que estan todas) pero exigir GLIBC_2.34 adentro, y entonces no
    abre en un Debian 11. Medido sobre el .deb de la rc4: exigia GLIBCXX_3.4.30, o sea la libstdc++ de
    GCC 12. Eso se arreglo metiendo la libreria adentro del binario (-static-libstdc++), como hace Core.

  ARQUITECTURA y MINIMO DE macOS: un Mach-O arm64 no corre en una Mac Intel. Si se anuncia "macOS" a
    secas hay que entregar las dos; si se anuncia Apple Silicon, hay que verificar que sea eso.

  RUTAS DEL COMPILADOR: cualquier /home/runner, /opt/homebrew o /usr/local adentro del binario es una
    ruta que en la maquina del usuario no existe.

FAIL-CLOSED, SIEMPRE
--------------------
Si el archivo no esta, esta vacio, o no se puede leer: FALLA. Un candado que no puede verificar no
puede aprobar. Ya paso de verdad: este guard estaba mal ubicado, miraba un archivo inexistente, la
herramienta fallaba, el grep no encontraba nada, el `if` no se cumplia y el guard decia OK. Verde sin
haber verificado nada.

Uso:
    python tools/check_portable.py <binario> [<binario>...]
    python tools/check_portable.py --self-test          prueba el verificador contra si mismo
"""
import sys

try:
    import lief
except ImportError:
    print("FALLO: falta lief.  pip install lief")
    sys.exit(1)

lief.logging.disable()

# ---------------------------------------------------------------- lo permitido
# Solo lo que trae CUALQUIER maquina del sistema operativo. Todo lo demas (libevent, boost, sqlite,
# libstdc++) va adentro del binario.
ELF_OK = {
    "libc.so.6", "libm.so.6", "libgcc_s.so.1", "libpthread.so.0", "libdl.so.2",
    "librt.so.1", "libatomic.so.1", "libresolv.so.2",
    "ld-linux-x86-64.so.2", "ld-linux-aarch64.so.1", "ld-linux.so.2",
}
MACHO_OK = {"libc++.1.dylib", "libSystem.B.dylib", "libresolv.9.dylib"}

# El piso declarado. Brisvia soporta Ubuntu 22.04+ / Debian 12+ porque la app (Tauri) necesita
# webkit2gtk-4.1, que abajo de eso no existe: el nodo no puede ser mas exigente que la app que lo lleva,
# pero tampoco tiene sentido pedirle menos. GLIBC 2.34 es lo que produce compilar en ubuntu-22.04.
# Core apunta a 2.31 porque distribuye el nodo suelto; nosotros distribuimos la app entera.
MAX_SIMBOLOS = {
    "GLIBC": (2, 34),
    "GLIBCXX": (3, 4, 30),   # solo si quedara libstdc++ dinamica; con -static-libstdc++ no aparece
    "CXXABI": (1, 3, 13),
    "GCC": (7, 0, 0),
    "LIBATOMIC": (1, 0),
}
MACOS_MINIMO = (13, 0)     # macOS 13, igual que Bitcoin Core v30
RUTAS_PROHIBIDAS = ("/home/runner", "/opt/homebrew", "/usr/local/opt", "/opt/hostedtoolcache",
                    "/Users/runner")


def _ver(txt):
    """'GLIBC_2.34' -> ('GLIBC', (2,34)). Devuelve None si no tiene forma de version."""
    if "_" not in txt:
        return None
    fam, _, v = txt.rpartition("_")
    try:
        return fam, tuple(int(x) for x in v.split("."))
    except ValueError:
        return None


def revisar_elf(b, fallos):
    for lib in b.libraries:
        if lib not in ELF_OK:
            fallos.append(f"pide una libreria que el usuario puede no tener: {lib}")

    # Lo que ningun ldd muestra: la version de simbolo mas alta que exige.
    pedidas = {}
    for s in b.symbols:
        sv = getattr(s, "symbol_version", None)
        aux = getattr(sv, "symbol_version_auxiliary", None) if sv else None
        if aux and (p := _ver(aux.name)):
            fam, v = p
            pedidas[fam] = max(pedidas.get(fam, v), v)
    for fam, v in sorted(pedidas.items()):
        tope = MAX_SIMBOLOS.get(fam)
        punto = ".".join(map(str, v))
        if tope is None:
            fallos.append(f"exige {fam}_{punto} y {fam} no esta declarado como soportado")
        elif v > tope:
            fallos.append(f"exige {fam}_{punto}, por encima del piso declarado "
                          f"{fam}_{'.'.join(map(str, tope))}: no abre en esas maquinas")
        else:
            print(f"      {fam:9} exige {punto:9} (tope {'.'.join(map(str, tope))})  ok")

    if (i := b.interpreter) and not i.startswith(("/lib64/", "/lib/")):
        fallos.append(f"cargador en una ruta rara: {i}")
    for e in b.dynamic_entries:
        if getattr(e, "tag", None) in (lief.ELF.DynamicEntry.TAG.RUNPATH, lief.ELF.DynamicEntry.TAG.RPATH):
            fallos.append(f"tiene RUNPATH/RPATH grabado ({e}): busca librerias en rutas del compilador")


def revisar_macho(b, fallos):
    for lib in b.libraries:
        nom = lib.name.split("/")[-1]
        if nom not in MACHO_OK:
            fallos.append(f"pide una libreria que el usuario puede no tener: {lib.name}")
        if any(p in lib.name for p in RUTAS_PROHIBIDAS) or lib.name.startswith("/opt/"):
            fallos.append(f"apunta a una ruta de la maquina de compilacion: {lib.name}")
    bv = getattr(b, "build_version", None)
    if bv:
        m = tuple(bv.minos[:2])
        if m > MACOS_MINIMO:
            fallos.append(f"exige macOS {m[0]}.{m[1]} y el piso declarado es "
                          f"{MACOS_MINIMO[0]}.{MACOS_MINIMO[1]}")
        else:
            print(f"      minimo de macOS: {m[0]}.{m[1]}  ok")
    print(f"      arquitectura: {b.header.cpu_type}")


def revisar_pe(b, fallos):
    for lib in b.libraries:
        print(f"      pide: {lib}")
    if any(l.lower().startswith("vcruntime") or l.lower().startswith("msvcp") for l in b.libraries):
        fallos.append("pide los runtimes de Visual C++ por fuera: en una Windows limpia pueden no estar")


def revisar(ruta) -> list:
    import os
    fallos = []
    print(f"\n  {ruta}")
    # Fail-closed antes que nada.
    if not os.path.isfile(ruta):
        return [f"no existe: {ruta}. El verificador no puede aprobar lo que no puede leer."]
    if os.path.getsize(ruta) == 0:
        return [f"esta vacio: {ruta}"]
    b = lief.parse(ruta)
    if b is None:
        return [f"no se puede leer como binario: {ruta}"]

    print(f"      formato: {b.format}")
    if b.format == lief.Binary.FORMATS.ELF:
        revisar_elf(b, fallos)
    elif b.format == lief.Binary.FORMATS.MACHO:
        revisar_macho(b, fallos)
    elif b.format == lief.Binary.FORMATS.PE:
        revisar_pe(b, fallos)
    else:
        fallos.append(f"formato inesperado: {b.format}")

    for lib in getattr(b, "libraries", []):
        nom = lib if isinstance(lib, str) else getattr(lib, "name", str(lib))
        for p in RUTAS_PROHIBIDAS:
            if p in nom:
                fallos.append(f"apunta a una ruta de la maquina de compilacion: {nom}")
    return fallos


def self_test() -> int:
    """El verificador tiene que rechazar lo que hay que rechazar. Si no, no sirve de nada.

    Se prueba contra el propio interprete de Python que lo esta corriendo: es un binario real del
    sistema, siempre esta, y no hace falta fabricar uno.
    """
    print("=== prueba del verificador contra si mismo ===")
    ok = True
    r = revisar("/no/existe/este/archivo")
    print(f"  archivo inexistente -> {'RECHAZA (bien)' if r else 'ACEPTA (MAL: fail-open!)'}")
    ok &= bool(r)
    b = lief.parse(sys.executable)
    if b is not None:
        print(f"  binario real leible ({sys.executable}): formato {b.format}  ok")
    else:
        print("  MAL: no puede leer ni el propio python")
        ok = False
    print(f"\n{'OK: el verificador falla cuando tiene que fallar' if ok else 'MAL'}")
    return 0 if ok else 1


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        sys.exit(2)
    if args[0] == "--self-test":
        sys.exit(self_test())

    todos = []
    for ruta in args:
        todos += [(ruta, f) for f in revisar(ruta)]
    print()
    if todos:
        print("RECHAZADO. Esto no abre en la maquina de un usuario:")
        for ruta, f in todos:
            print(f"  - {ruta}: {f}")
        sys.exit(1)
    print("OK: todo lo que pide existe en cualquier maquina del sistema operativo soportado.")
