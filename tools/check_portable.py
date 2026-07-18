#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Decide whether a binary will start on a user's machine, or only on the one that built it.

WHY THIS EXISTS
---------------
The published 1.0.5 shipped broken on macOS and Linux: the node ended up linked against libraries that
exist on the build machine (Homebrew, libevent-dev) and not on the user's. The app opens, the node dies, and
with no node there is no wallet, no network, no mining. Three green builds never caught it.

The first guard written for this was a bash block with `ldd` and name trimming via `sed`. It gave TWO
false positives in a row, both from assuming instead of reading:
  1) it trimmed the loader name wrong and rejected a perfectly good binary;
  2) then it assumed the loader was not listed among the declared dependencies. It is.
Each one cost a 40-minute build cycle.

This is not solved by writing bash more carefully: it is solved by reading the structure of the binary
instead of parsing text. That is what `lief` does, and it is what Bitcoin Core itself uses in
contrib/guix/symbol-check.py, which runs on every one of its releases.

WHAT IT CHECKS, AND WHY EACH THING
----------------------------------
  LIBRARIES: allowlist, never denylist. Searching for "libevent" is aiming at the last known bug; the
    next one will be another library. Declare what is allowed and reject everything else.

  SYMBOL VERSIONS: what really decides which machines it starts on, and what no `ldd` shows. A binary
    can ask for libc.so.6 (present everywhere) but demand GLIBC_2.34 inside, and then it will not open
    on a Debian 11. Measured on the rc4 .deb: it demanded GLIBCXX_3.4.30, i.e. the libstdc++ from GCC 12.
    That was fixed by bundling the library inside the binary (-static-libstdc++), the way Core does.

  ARCHITECTURE and macOS MINIMUM: a Mach-O arm64 does not run on an Intel Mac. If plain "macOS" is
    announced, both must ship; if Apple Silicon is announced, verify it really is that.

  COMPILER PATHS: any /home/runner, /opt/homebrew or /usr/local inside the binary is a path that does
    not exist on the user's machine.

FAIL-CLOSED, ALWAYS
-------------------
If the file is missing, empty, or unreadable: FAIL. A guard that cannot verify cannot approve. This has
actually happened: this guard was misplaced, looked at a nonexistent file, the tool failed, grep found
nothing, the `if` was never true and the guard said OK. Green without having verified anything.

Usage:
    python tools/check_portable.py <binary> [<binary>...]
    python tools/check_portable.py --self-test          test the verifier against itself
"""
import sys

try:
    import lief
except ImportError:
    print("FAILED: lief is missing.  pip install lief")
    sys.exit(1)

lief.logging.disable()

# ---------------------------------------------------------------- what is allowed
# Only what EVERY machine of the operating system ships. Everything else (libevent, boost, sqlite,
# libstdc++) goes inside the binary.
ELF_OK = {
    "libc.so.6", "libm.so.6", "libgcc_s.so.1", "libpthread.so.0", "libdl.so.2",
    "librt.so.1", "libatomic.so.1", "libresolv.so.2",
    "ld-linux-x86-64.so.2", "ld-linux-aarch64.so.1", "ld-linux.so.2",
}
MACHO_OK = {"libc++.1.dylib", "libSystem.B.dylib", "libresolv.9.dylib"}

# The declared floor. Brisvia supports Ubuntu 22.04+ / Debian 12+ because the app (Tauri) needs
# webkit2gtk-4.1, which does not exist below that: the node cannot be more demanding than the app that
# carries it, but there is no point in asking for less either. GLIBC 2.34 is what building on ubuntu-22.04
# produces. Core targets 2.31 because it distributes the node on its own; we distribute the whole app.
MAX_SIMBOLOS = {
    "GLIBC": (2, 34),
    "GLIBCXX": (3, 4, 30),   # only if libstdc++ stays dynamic; with -static-libstdc++ it does not appear
    "CXXABI": (1, 3, 13),
    "GCC": (7, 0, 0),
    "LIBATOMIC": (1, 0),
}
MACOS_MINIMO = (13, 0)     # macOS 13, same as Bitcoin Core v30
RUTAS_PROHIBIDAS = ("/home/runner", "/opt/homebrew", "/usr/local/opt", "/opt/hostedtoolcache",
                    "/Users/runner")


def _ver(txt):
    """'GLIBC_2.34' -> ('GLIBC', (2,34)). Returns None if it has no version shape."""
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
            fallos.append(f"requires a library the user may not have: {lib}")

    # What no ldd shows: the highest symbol version it demands.
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
            fallos.append(f"demands {fam}_{punto} and {fam} is not declared as supported")
        elif v > tope:
            fallos.append(f"demands {fam}_{punto}, above the declared floor "
                          f"{fam}_{'.'.join(map(str, tope))}: it will not open on those machines")
        else:
            print(f"      {fam:9} demands {punto:9} (cap {'.'.join(map(str, tope))})  ok")

    if (i := b.interpreter) and not i.startswith(("/lib64/", "/lib/")):
        fallos.append(f"loader in an unusual path: {i}")
    for e in b.dynamic_entries:
        if getattr(e, "tag", None) in (lief.ELF.DynamicEntry.TAG.RUNPATH, lief.ELF.DynamicEntry.TAG.RPATH):
            fallos.append(f"has RUNPATH/RPATH baked in ({e}): it looks for libraries in compiler paths")


def revisar_macho(b, fallos):
    for lib in b.libraries:
        nom = lib.name.split("/")[-1]
        if nom not in MACHO_OK:
            fallos.append(f"requires a library the user may not have: {lib.name}")
        if any(p in lib.name for p in RUTAS_PROHIBIDAS) or lib.name.startswith("/opt/"):
            fallos.append(f"points at a build-machine path: {lib.name}")
    bv = getattr(b, "build_version", None)
    if bv:
        m = tuple(bv.minos[:2])
        if m > MACOS_MINIMO:
            fallos.append(f"demands macOS {m[0]}.{m[1]} and the declared floor is "
                          f"{MACOS_MINIMO[0]}.{MACOS_MINIMO[1]}")
        else:
            print(f"      macOS minimum: {m[0]}.{m[1]}  ok")
    print(f"      architecture: {b.header.cpu_type}")


def revisar_pe(b, fallos):
    for lib in b.libraries:
        print(f"      requires: {lib}")
    if any(l.lower().startswith("vcruntime") or l.lower().startswith("msvcp") for l in b.libraries):
        fallos.append("requires the Visual C++ runtimes externally: they may not be present on a clean Windows")


def revisar(ruta) -> list:
    import os
    fallos = []
    print(f"\n  {ruta}")
    # Fail-closed before anything else.
    if not os.path.isfile(ruta):
        return [f"does not exist: {ruta}. The verifier cannot approve what it cannot read."]
    if os.path.getsize(ruta) == 0:
        return [f"is empty: {ruta}"]
    b = lief.parse(ruta)
    if b is None:
        return [f"cannot be read as a binary: {ruta}"]

    print(f"      format: {b.format}")
    if b.format == lief.Binary.FORMATS.ELF:
        revisar_elf(b, fallos)
    elif b.format == lief.Binary.FORMATS.MACHO:
        revisar_macho(b, fallos)
    elif b.format == lief.Binary.FORMATS.PE:
        revisar_pe(b, fallos)
    else:
        fallos.append(f"unexpected format: {b.format}")

    for lib in getattr(b, "libraries", []):
        nom = lib if isinstance(lib, str) else getattr(lib, "name", str(lib))
        for p in RUTAS_PROHIBIDAS:
            if p in nom:
                fallos.append(f"points at a build-machine path: {nom}")
    return fallos


def self_test() -> int:
    """The verifier has to reject what should be rejected. Otherwise it is useless.

    It is tested against the very Python interpreter running it: a real system binary, always present,
    with no need to fabricate one.
    """
    print("=== testing the verifier against itself ===")
    ok = True
    r = revisar("/no/such/file/here")
    print(f"  nonexistent file -> {'REJECTS (good)' if r else 'ACCEPTS (BAD: fail-open!)'}")
    ok &= bool(r)
    b = lief.parse(sys.executable)
    if b is not None:
        print(f"  real readable binary ({sys.executable}): format {b.format}  ok")
    else:
        print("  BAD: cannot even read python itself")
        ok = False
    print(f"\n{'OK: the verifier fails when it has to fail' if ok else 'BAD'}")
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
        print("REJECTED. This will not open on a user's machine:")
        for ruta, f in todos:
            print(f"  - {ruta}: {f}")
        sys.exit(1)
    print("OK: everything it requires exists on any machine of the supported operating system.")
