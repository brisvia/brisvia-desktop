"""Read the import table of a PE by name AND by ordinal. Names alone are half the check.

WHY
---
I told ChatGPT "no symbol is missing" from the rc6 test executable, and it refused the claim:

    "Your claim 'all symbols exist' must be broadened to: all names, all ordinals and all final targets
     of forwarded exports. Until then, the contradiction is still not complete."

It is right. I compared names out of dumpbin's text output. An import by ORDINAL has no name to compare,
so a name-only diff reports it as absent rather than as unmatched -- it disappears instead of failing.
And an export can be a FORWARDER: the name is present in the first DLL and merely points somewhere else,
which a name check reads as satisfied.

It also brought a precedent: windows-rs hit exactly this 0xC0000139 because GetWindowSubclass got linked
by ordinal against COMCTL32 instead of by name.

This reads the PE structures directly rather than parsing dumpbin's prose, because the text output does
not distinguish "imported by ordinal 410" from a name it failed to print.

    python check_imports_deep.py test-bad.exe
    python check_imports_deep.py --self-test
"""
import argparse
import pathlib
import struct
import sys


def _u16(b, o):
    return struct.unpack_from("<H", b, o)[0]


def _u32(b, o):
    return struct.unpack_from("<I", b, o)[0]


def _u64(b, o):
    return struct.unpack_from("<Q", b, o)[0]


def _cstr(b, o):
    e = b.find(b"\0", o)
    return b[o:e].decode("ascii", "replace")


class PE:
    def __init__(self, data):
        self.b = data
        if data[:2] != b"MZ":
            raise ValueError("not a PE: no MZ")
        pe = _u32(data, 0x3C)
        if data[pe:pe + 4] != b"PE\0\0":
            raise ValueError("not a PE: no PE signature")
        self.pe = pe
        self.nsec = _u16(data, pe + 6)
        opt = pe + 24
        self.magic = _u16(data, opt)
        self.pe32plus = self.magic == 0x20B
        # The data directory sits after the optional header's fixed part, whose size differs by magic.
        dd = opt + (112 if self.pe32plus else 96)
        self.import_rva = _u32(data, dd + 8)
        self.sections = []
        so = opt + _u16(data, pe + 20)
        for i in range(self.nsec):
            s = so + i * 40
            self.sections.append((_u32(data, s + 12), _u32(data, s + 8), _u32(data, s + 20),
                                  _u32(data, s + 16)))

    def off(self, rva):
        """RVA -> file offset. Returns None when it lands outside every section."""
        for vaddr, vsize, praw, sraw in self.sections:
            if vaddr <= rva < vaddr + max(vsize, sraw):
                d = rva - vaddr
                if d < sraw:
                    return praw + d
        return None

    def imports(self):
        """[(dll, [(kind, value)])] where kind is 'name' or 'ordinal'."""
        out = []
        if not self.import_rva:
            return out
        o = self.off(self.import_rva)
        if o is None:
            return out
        while True:
            oft, _, _, name_rva, first = struct.unpack_from("<IIIII", self.b, o)
            if not (oft or first or name_rva):
                break
            no = self.off(name_rva)
            dll = _cstr(self.b, no) if no is not None else "?"
            simbolos = []
            t = self.off(oft or first)
            if t is not None:
                paso = 8 if self.pe32plus else 4
                leer = _u64 if self.pe32plus else _u32
                bit = 1 << (63 if self.pe32plus else 31)
                while True:
                    v = leer(self.b, t)
                    if v == 0:
                        break
                    if v & bit:
                        # THE case a name-only diff cannot see: no name exists, only a number.
                        simbolos.append(("ordinal", v & 0xFFFF))
                    else:
                        h = self.off(v & 0x7FFFFFFF)
                        simbolos.append(("name", _cstr(self.b, h + 2) if h is not None else "?"))
                    t += paso
            out.append((dll, simbolos))
            o += 20
        return out


def self_test():
    """Point it at a real Windows binary and assert it reads something sane."""
    fallos = 0
    for cand in (r"C:\Windows\System32\notepad.exe", r"C:\Windows\System32\cmd.exe"):
        p = pathlib.Path(cand)
        if not p.exists():
            continue
        try:
            imp = PE(p.read_bytes()).imports()
        except Exception as e:
            print(f"  FAIL  could not read {p.name}: {e}")
            fallos += 1
            continue
        total = sum(len(s) for _, s in imp)
        if not imp or total < 5:
            print(f"  FAIL  {p.name}: {len(imp)} dlls / {total} symbols -- too little to be real")
            fallos += 1
        else:
            print(f"  PASS  reads-a-real-PE  ({p.name}: {len(imp)} dlls, {total} symbols)")
        break
    else:
        print("  FAIL  found no Windows binary to test")
        fallos += 1

    try:
        PE(b"not a PE" + b"\0" * 200)
        print("  FAIL  accepted something that is not a PE")
        fallos += 1
    except ValueError:
        print("  PASS  rejects-what-is-not-a-PE")

    print()
    if fallos:
        print(f"SELF-TEST FAILED ({fallos})")
        return 1
    print("self-test OK")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("exe", nargs="?")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.exe:
        return ap.error("an .exe path is required")

    imp = PE(pathlib.Path(a.exe).read_bytes()).imports()
    print(f"{a.exe}")
    print(f"  {len(imp)} DLLs, {sum(len(s) for _, s in imp)} imports")
    print()

    por_ordinal = []
    for dll, simbolos in imp:
        ords = [v for k, v in simbolos if k == "ordinal"]
        marca = f"  <-- {len(ords)} BY ORDINAL: {ords}" if ords else ""
        print(f"  {dll:<22} {len(simbolos):>4} imports{marca}")
        if ords:
            por_ordinal.append((dll, ords))

    print()
    if por_ordinal:
        print("  IMPORTS BY ORDINAL FOUND. A by-name diff does NOT see them:")
        for dll, ords in por_ordinal:
            print(f"    {dll}: {ords}")
        print("  Each one must be resolved against the /EXPORTS of the DLL Windows actually loads.")
    else:
        print("  Zero imports by ordinal. That path is ruled out by reading the PE directly,")
        print("  not by failing to look.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
