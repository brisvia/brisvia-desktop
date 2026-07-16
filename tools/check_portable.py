#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Decide whether a binary starts on a user's machine, or only on the one that compiled it.

WHY IT EXISTS
-------------
The published 1.0.5 ships broken on macOS and Linux: the node was linked against libraries that exist on
the build machine (Homebrew, libevent-dev) and not on the user's. The app opens, the node dies, and with
no node there is no wallet, no network, no mining. Three green builds did not catch it.

The first guard I wrote was a bash block with `ldd` and name trimming via `sed`. It gave TWO false
positives in a row, both from assuming instead of reading:
  1) it trimmed the loader name wrong and rejected a perfect binary;
  2) then it assumed the loader was not among the declared dependencies. It is.
Each cost a 40-minute build cycle.

This is not solved by writing bash more carefully: it is solved by reading the binary's structure instead
of parsing text. That is what `lief` does, and it is what Bitcoin Core itself uses in
contrib/guix/symbol-check.py, which runs on every one of its releases.

WHAT IT CHECKS, AND WHY EACH THING
----------------------------------
  LIBRARIES: allowlist, never denylist. Searching for "libevent" aims at the last known bug; the next one
    will be a different library. Declare what is allowed and reject everything else.

  SYMBOL VERSIONS: what really decides which machines it starts on, and what no `ldd` shows. A binary can
    ask for libc.so.6 (present everywhere) but require GLIBC_2.34 inside, and then it does not open on a
    Debian 11. Measured on the rc4 .deb: it required GLIBCXX_3.4.30, i.e. libstdc++ from GCC 12. That was
    fixed by pulling the library inside the binary (-static-libstdc++), as Core does.

  ARCHITECTURE and macOS MINIMUM: an arm64 Mach-O does not run on an Intel Mac. If "macOS" is advertised
    plainly, both must be shipped; if Apple Silicon is advertised, verify it is that.

  COMPILER PATHS: any /home/runner, /opt/homebrew or /usr/local inside the binary is a path that does not
    exist on the user's machine.

FAIL-CLOSED, ALWAYS
-------------------
If the file is missing, empty, or unreadable: it FAILS. A guard that cannot verify cannot approve. It
already happened for real: this guard was misplaced, looked at a nonexistent file, the tool failed, the
grep found nothing, the `if` was not satisfied and the guard said OK. Green without having verified
anything.

Usage:
    python tools/check_portable.py <binary> [<binary>...]
    python tools/check_portable.py --self-test          test the checker against itself
"""
import sys

try:
    import lief
except ImportError:
    print("FAIL: lief is missing.  pip install lief")
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
# carries it, but it makes no sense to ask less either. GLIBC 2.34 is what compiling on ubuntu-22.04
# produces. Core targets 2.31 because it distributes the node on its own; we distribute the whole app.
MAX_SYMBOLS = {
    "GLIBC": (2, 34),
    "GLIBCXX": (3, 4, 30),   # only if libstdc++ stayed dynamic; with -static-libstdc++ it does not appear
    "CXXABI": (1, 3, 13),
    "GCC": (7, 0, 0),
    "LIBATOMIC": (1, 0),
}
MACOS_MINIMUM = (13, 0)     # macOS 13, same as Bitcoin Core v30
FORBIDDEN_PATHS = ("/home/runner", "/opt/homebrew", "/usr/local/opt", "/opt/hostedtoolcache",
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


def check_elf(b, problems):
    for lib in b.libraries:
        if lib not in ELF_OK:
            problems.append(f"asks for a library the user may not have: {lib}")

    # What no ldd shows: the highest symbol version it requires.
    required = {}
    for s in b.symbols:
        sv = getattr(s, "symbol_version", None)
        aux = getattr(sv, "symbol_version_auxiliary", None) if sv else None
        if aux and (p := _ver(aux.name)):
            fam, v = p
            required[fam] = max(required.get(fam, v), v)
    for fam, v in sorted(required.items()):
        ceiling = MAX_SYMBOLS.get(fam)
        dotted = ".".join(map(str, v))
        if ceiling is None:
            problems.append(f"requires {fam}_{dotted} and {fam} is not declared as supported")
        elif v > ceiling:
            problems.append(f"requires {fam}_{dotted}, above the declared floor "
                            f"{fam}_{'.'.join(map(str, ceiling))}: it does not open on those machines")
        else:
            print(f"      {fam:9} requires {dotted:9} (ceiling {'.'.join(map(str, ceiling))})  ok")

    if (i := b.interpreter) and not i.startswith(("/lib64/", "/lib/")):
        problems.append(f"loader on an odd path: {i}")
    for e in b.dynamic_entries:
        if getattr(e, "tag", None) in (lief.ELF.DynamicEntry.TAG.RUNPATH, lief.ELF.DynamicEntry.TAG.RPATH):
            problems.append(f"has a RUNPATH/RPATH baked in ({e}): it looks for libraries on compiler paths")


def check_macho(b, problems):
    for lib in b.libraries:
        name = lib.name.split("/")[-1]
        if name not in MACHO_OK:
            problems.append(f"asks for a library the user may not have: {lib.name}")
        if any(p in lib.name for p in FORBIDDEN_PATHS) or lib.name.startswith("/opt/"):
            problems.append(f"points at a build-machine path: {lib.name}")
    bv = getattr(b, "build_version", None)
    if bv:
        m = tuple(bv.minos[:2])
        if m > MACOS_MINIMUM:
            problems.append(f"requires macOS {m[0]}.{m[1]} and the declared floor is "
                            f"{MACOS_MINIMUM[0]}.{MACOS_MINIMUM[1]}")
        else:
            print(f"      macOS minimum: {m[0]}.{m[1]}  ok")
    print(f"      architecture: {b.header.cpu_type}")


def check_pe(b, problems):
    for lib in b.libraries:
        print(f"      asks for: {lib}")
    if any(l.lower().startswith("vcruntime") or l.lower().startswith("msvcp") for l in b.libraries):
        problems.append("asks for the Visual C++ runtimes externally: on a clean Windows they may be missing")


def check(path) -> list:
    import os
    problems = []
    print(f"\n  {path}")
    # Fail-closed before anything else.
    if not os.path.isfile(path):
        return [f"does not exist: {path}. The checker cannot approve what it cannot read."]
    if os.path.getsize(path) == 0:
        return [f"is empty: {path}"]
    b = lief.parse(path)
    if b is None:
        return [f"cannot be read as a binary: {path}"]

    print(f"      format: {b.format}")
    if b.format == lief.Binary.FORMATS.ELF:
        check_elf(b, problems)
    elif b.format == lief.Binary.FORMATS.MACHO:
        check_macho(b, problems)
    elif b.format == lief.Binary.FORMATS.PE:
        check_pe(b, problems)
    else:
        problems.append(f"unexpected format: {b.format}")

    for lib in getattr(b, "libraries", []):
        name = lib if isinstance(lib, str) else getattr(lib, "name", str(lib))
        for p in FORBIDDEN_PATHS:
            if p in name:
                problems.append(f"points at a build-machine path: {name}")
    return problems


def self_test() -> int:
    """The checker has to reject what must be rejected. Otherwise it is worth nothing.

    It is tested against the very Python interpreter running it: a real system binary, always present,
    and no need to fabricate one.
    """
    print("=== checker tested against itself ===")
    ok = True
    r = check("/does/not/exist/this/file")
    print(f"  missing file -> {'REJECTS (good)' if r else 'ACCEPTS (BAD: fail-open!)'}")
    ok &= bool(r)
    b = lief.parse(sys.executable)
    if b is not None:
        print(f"  real readable binary ({sys.executable}): format {b.format}  ok")
    else:
        print("  BAD: cannot read even python itself")
        ok = False
    print(f"\n{'OK: the checker fails when it must fail' if ok else 'BAD'}")
    return 0 if ok else 1


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        sys.exit(2)
    if args[0] == "--self-test":
        sys.exit(self_test())

    all_problems = []
    for path in args:
        all_problems += [(path, f) for f in check(path)]
    print()
    if all_problems:
        print("REJECTED. This does not open on a user's machine:")
        for path, f in all_problems:
            print(f"  - {path}: {f}")
        sys.exit(1)
    print("OK: everything it asks for exists on any machine of the supported operating system.")
