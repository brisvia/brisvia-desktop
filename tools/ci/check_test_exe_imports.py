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

FORBIDDEN = {
    "TaskDialogIndirect": (
        "Common Controls v6 only. A manifest-less cargo test binary gets comctl32 5.82, which does not "
        "export it: 0xC0000139 before main, zero tests run."
    ),
    "TaskDialog": "Common Controls v6 only. Same failure as TaskDialogIndirect.",
}

# comctl32 in the test binary means Tauri's GUI runtime got linked in, which is the coupling itself.
FORBIDDEN_DLLS = {"comctl32.dll"}


def read_text(path):
    """Decode a file whichever way it was written. The false negative that let this bug hide.

    The comparison that was supposed to catch the missing symbol read dumpbin's output as UTF-8. PowerShell's
    Out-File writes UTF-16 LE with a BOM, so it searched mojibake, found nothing, and reported 'no symbol
    is missing'. It was there the whole time.
    """
    b = pathlib.Path(path).read_bytes()
    for bom, enc in ((b"\xff\xfe\x00\x00", "utf-32"), (b"\x00\x00\xfe\xff", "utf-32"),
                     (b"\xff\xfe", "utf-16-le"), (b"\xfe\xff", "utf-16-be"), (b"\xef\xbb\xbf", "utf-8-sig")):
        if b.startswith(bom):
            return b.decode(enc, errors="replace")
    # No BOM. UTF-16 without one still gives itself away: ASCII text becomes alternating NUL bytes.
    sample = b[:400]
    if sample.count(b"\x00") > len(sample) // 4:
        pairs = sample[:40]
        return b.decode("utf-16-le" if pairs[1::2].count(0) > pairs[0::2].count(0) else "utf-16-be",
                        errors="replace")
    return b.decode("utf-8", errors="replace")


def imports_of(exe):
    """The PE import table, by name and by ordinal. Reuses the reader that proved the root cause."""
    import importlib.util
    here = pathlib.Path(__file__).parent / "check_imports_deep.py"
    spec = importlib.util.spec_from_file_location("deep", here)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m.PE(pathlib.Path(exe).read_bytes()).imports()


def check(exe):
    """Returns a list of failures. Empty means clean."""
    failures = []
    p = pathlib.Path(exe)
    if not p.exists():
        return [f"no such executable: {exe}. Fail-closed: nothing was inspected."]
    try:
        imp = imports_of(exe)
    except Exception as e:
        return [f"could not read the PE import table ({e}). Fail-closed: nothing was proven."]
    if not imp:
        return [f"{p.name} has no import table at all. Fail-closed: that is not a normal test binary."]

    for dll, symbols in imp:
        names = {v for k, v in symbols if k == "name"}
        if dll.lower() in FORBIDDEN_DLLS:
            failures.append(
                f"{p.name} imports {dll}. In the TEST executable that means Tauri's GUI runtime got "
                f"linked in -- the coupling that caused 0xC0000139. (The app itself may use it; this "
                f"gate is only about cargo's test binary.)")
        for forbidden, reason in FORBIDDEN.items():
            if forbidden in names:
                failures.append(f"{p.name} imports {forbidden} from {dll}. {reason}")
    return failures


def self_test():
    """A gate nobody has seen fail is not a gate."""
    failures = 0

    for path, reason in (("does-not-exist-anywhere.exe", "a missing file"),
                         (__file__, "a file that is not a PE")):
        if not check(path):
            print(f"  FAIL  passed on {reason} -- fail-closed means the opposite")
            failures += 1
        else:
            print(f"  PASS  fails-closed-on-{reason.replace(' ', '-')}")

    # A real binary that imports none of the forbidden things must pass, or the gate is just noise.
    clean = r"C:\Windows\System32\cmd.exe"
    if pathlib.Path(clean).exists():
        r = check(clean)
        if r:
            print(f"  FAIL  rejected a clean binary: {r[0][:70]}")
            failures += 1
        else:
            print("  PASS  accepts-a-clean-binary")

    # THE encoding false negative, pinned. UTF-16 LE with a BOM is what Out-File writes.
    import tempfile
    for enc, name in (("utf-16-le", "utf-16-le-with-bom"), ("utf-16-be", "utf-16-be-with-bom"),
                      ("utf-8-sig", "utf-8-with-bom"), ("utf-8", "plain-utf-8")):
        with tempfile.NamedTemporaryFile(suffix=".txt", delete=False) as f:
            text = "  ordinal hint RVA      name\n  120  TaskDialogIndirect\n"
            bom = {"utf-16-le": b"\xff\xfe", "utf-16-be": b"\xfe\xff"}.get(enc, b"")
            f.write(bom + text.encode(enc if enc != "utf-8-sig" else "utf-8-sig"))
            path = f.name
        read_content = read_text(path)
        pathlib.Path(path).unlink()
        if "TaskDialogIndirect" not in read_content:
            print(f"  FAIL  {name}: TaskDialogIndirect was there and the reader did not see it -- "
                  f"exactly the false negative that hid this bug")
            failures += 1
        else:
            print(f"  PASS  reads-{name}")

    print()
    if failures:
        print(f"SELF-TEST FAILED ({failures})")
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

    failures = check(a.exe)
    if failures:
        print(f"FAIL: {a.exe}")
        for f in failures:
            print(f"  {f}")
        return 1
    print(f"OK: {pathlib.Path(a.exe).name} imports nothing forbidden.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
