"""Fail-closed gate: the test executable must never import TaskDialogIndirect again.

WHAT IT GUARDS, AND WHY IT IS PERMANENT
--------------------------------------
rc6's node_shutdown_tests built an AppState to call the shutdown helper. AppState carries
`tray: Arc<Mutex<Option<tauri::tray::TrayIcon>>>`, so constructing it made the TrayIcon type reachable,
which linked Tauri's GUI runtime into the TEST binary, which imported TaskDialogIndirect from
comctl32.dll. That function exists only in Common Controls v6. A cargo test binary carries no manifest,
so Windows hands it C:\\Windows\\System32\\comctl32.dll v5.82 -- sha256
2BBCAA9135A9A96C9E377D71EB40D5ECF5AEE7BD8E0B0ECB57405B650B78F3AD -- which does not export it. The
executable died with 0xC0000139 before main. Zero tests ran, and three green builds shipped anyway.

Proven both ways in run 29465731970:
    rc6-intact          exit=-1073741511  0xC0000139   0 tests   imports TaskDialogIndirect
    rc6-minimal-helper  exit=0            0x00000000  61 tests   imports no comctl32 at all
    rc5-control         exit=0            0x00000000  58 tests   imports no comctl32 at all

The fix is architectural -- the helper takes a process slot, not an AppState -- and nothing in the
compiler stops someone reintroducing the coupling. A reviewer would see a reasonable-looking test that
builds an AppState. This gate sees the import.

FAIL-CLOSED
-----------
Anything it cannot prove is a failure: no executable, unreadable PE, no import table. A gate that passes
when it cannot see is not a gate. The whole reason this bug shipped is that a green result meant "nothing
looked", and there is no version of that mistake worth repeating.

NOT A PRODUCT RULE
------------------
comctl32 is forbidden in the TEST executable, not in the app. The packaged app needs Tauri, TrayIcon,
and the GUI runtime -- it has a manifest and gets v6. This gate points only at cargo's test binary.

    python check_test_exe_imports.py path/to/test.exe
    python check_test_exe_imports.py --self-test
"""
import argparse
import pathlib
import sys

PROHIBIDOS = {
    "TaskDialogIndirect": (
        "Common Controls v6 only. A manifest-less cargo test binary gets comctl32 5.82, which does not "
        "export it: 0xC0000139 before main, zero tests run."
    ),
    "TaskDialog": "Common Controls v6 only. Same failure as TaskDialogIndirect.",
}

# comctl32 in the test binary means Tauri's GUI runtime got linked in, which is the coupling itself.
DLL_PROHIBIDAS = {"comctl32.dll"}


def leer_texto(ruta):
    """Decode a file whichever way it was written. The false negative that let this bug hide.

    The comparison that was supposed to catch the missing symbol read dumpbin's output as UTF-8. PowerShell's
    Out-File writes UTF-16 LE with a BOM, so it searched mojibake, found nothing, and reported 'no symbol
    is missing'. It was there the whole time.
    """
    b = pathlib.Path(ruta).read_bytes()
    for bom, enc in ((b"\xff\xfe\x00\x00", "utf-32"), (b"\x00\x00\xfe\xff", "utf-32"),
                     (b"\xff\xfe", "utf-16-le"), (b"\xfe\xff", "utf-16-be"), (b"\xef\xbb\xbf", "utf-8-sig")):
        if b.startswith(bom):
            return b.decode(enc, errors="replace")
    # No BOM. UTF-16 without one still gives itself away: ASCII text becomes alternating NUL bytes.
    muestra = b[:400]
    if muestra.count(b"\x00") > len(muestra) // 4:
        pares = muestra[:40]
        return b.decode("utf-16-le" if pares[1::2].count(0) > pares[0::2].count(0) else "utf-16-be",
                        errors="replace")
    return b.decode("utf-8", errors="replace")


def imports_de(exe):
    """The PE import table, by name and by ordinal. Reuses the reader that proved the root cause."""
    import importlib.util
    aqui = pathlib.Path(__file__).parent / "check_imports_deep.py"
    spec = importlib.util.spec_from_file_location("deep", aqui)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m.PE(pathlib.Path(exe).read_bytes()).imports()


def revisar(exe):
    """Returns a list of failures. Empty means clean."""
    fallos = []
    p = pathlib.Path(exe)
    if not p.exists():
        return [f"no such executable: {exe}. Fail-closed: nothing was inspected."]
    try:
        imp = imports_de(exe)
    except Exception as e:
        return [f"could not read the PE import table ({e}). Fail-closed: nothing was proven."]
    if not imp:
        return [f"{p.name} has no import table at all. Fail-closed: that is not a normal test binary."]

    for dll, simbolos in imp:
        nombres = {v for k, v in simbolos if k == "name"}
        if dll.lower() in DLL_PROHIBIDAS:
            fallos.append(
                f"{p.name} imports {dll}. In the TEST executable that means Tauri's GUI runtime got "
                f"linked in -- the coupling that caused 0xC0000139. (The app itself may use it; this "
                f"gate is only about cargo's test binary.)")
        for prohibido, porque in PROHIBIDOS.items():
            if prohibido in nombres:
                fallos.append(f"{p.name} imports {prohibido} from {dll}. {porque}")
    return fallos


def self_test():
    """A gate nobody has seen fail is not a gate."""
    fallos = 0

    for ruta, porque in (("no-existe-en-ningun-lado.exe", "a missing file"),
                         (__file__, "a file that is not a PE")):
        if not revisar(ruta):
            print(f"  FAIL  passed on {porque} -- fail-closed means the opposite")
            fallos += 1
        else:
            print(f"  PASS  fails-closed-on-{porque.replace(' ', '-')}")

    # A real binary that imports none of the forbidden things must pass, or the gate is just noise.
    limpio = r"C:\Windows\System32\cmd.exe"
    if pathlib.Path(limpio).exists():
        r = revisar(limpio)
        if r:
            print(f"  FAIL  rejected a clean binary: {r[0][:70]}")
            fallos += 1
        else:
            print("  PASS  accepts-a-clean-binary")

    # THE encoding false negative, pinned. UTF-16 LE with a BOM is what Out-File writes.
    import tempfile
    for enc, nombre in (("utf-16-le", "utf-16-le-with-bom"), ("utf-16-be", "utf-16-be-with-bom"),
                        ("utf-8-sig", "utf-8-with-bom"), ("utf-8", "plain-utf-8")):
        with tempfile.NamedTemporaryFile(suffix=".txt", delete=False) as f:
            texto = "  ordinal hint RVA      name\n  120  TaskDialogIndirect\n"
            bom = {"utf-16-le": b"\xff\xfe", "utf-16-be": b"\xfe\xff"}.get(enc, b"")
            f.write(bom + texto.encode(enc if enc != "utf-8-sig" else "utf-8-sig"))
            ruta = f.name
        leido = leer_texto(ruta)
        pathlib.Path(ruta).unlink()
        if "TaskDialogIndirect" not in leido:
            print(f"  FAIL  {nombre}: TaskDialogIndirect was there and the reader did not see it -- "
                  f"exactly the false negative that hid this bug")
            fallos += 1
        else:
            print(f"  PASS  reads-{nombre}")

    print()
    if fallos:
        print(f"SELF-TEST FAILED ({fallos})")
        return 1
    print("self-test OK: the gate fails closed and reads every encoding")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("exe", nargs="?")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.exe:
        return ap.error("an executable is required")

    fallos = revisar(a.exe)
    if fallos:
        print(f"FAIL: {a.exe}")
        for f in fallos:
            print(f"  {f}")
        return 1
    print(f"OK: {pathlib.Path(a.exe).name} imports nothing forbidden.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
