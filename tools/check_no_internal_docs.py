#!/usr/bin/env python3
"""Fail if an internal working document reaches the public repository.

WHY
---
Brisvia is a public coin. Its repository is read by strangers, and everything in it is a statement to
them. Internal working documents were in there that had no business being: an incident log with real
mistakes, procedures and internal patterns, and various planning and backlog notes. None of it is
documentation anyone outside needs. Nothing in the product read them.

They were removed from the tree, not from history: older tags still contain them and that stays.
Rewriting published history to hide that they existed would be worse than having published them.

This exists so they do not come back. A rule in a document does not stop anything -- that lesson is
already paid for in this project. If something must not be in the repo, a step has to FAIL when it is.

AN EXPLICIT LIST, NOT A PATTERN
-------------------------------
Deliberately a named list rather than something clever like "reject files that look internal". A broad
pattern would eventually block CONTRIBUTING.md or a legitimate design doc, and a guard that blocks
legitimate work gets switched off -- and then it guards nothing. Adding a file here should be a decision
somebody makes on purpose.

Usage:
    python tools/check_no_internal_docs.py
    python tools/check_no_internal_docs.py --self-test
"""
import subprocess
import sys

# Named one by one, each with the reason it does not belong to the public.
FORBIDDEN = {
    "INCIDENT_PATTERNS.md": "internal incident log: mistakes, procedures and team patterns",
    "PENDIENTES-SIN-PUBLICAR.md": "internal list of unreleased work; its own name says it is not public",
    "MEJORAS-PENDIENTES.md": "internal backlog",
    "LOGROS-50-DISENO.md": "internal design notes",
    "PLAN.md": "internal planning",
}

# The private copies live outside any git repository. The path is deliberately NOT written here: this
# file is public, and publishing the folder layout of a real machine helps nobody outside and helps
# anyone looking for a way in.


def tracked() -> set:
    r = subprocess.run(["git", "ls-files"], capture_output=True, text=True)
    return {l.replace("\\", "/") for l in r.stdout.splitlines() if l}


def scan(files: set) -> list:
    """Match by basename, anywhere in the tree, ignoring case.

    An internal document does not stop being internal by sitting in docs/, and Windows would happily
    let `plan.md` through a check that only knew about `PLAN.md`. Both were holes in the first
    version of this: it compared exact top-level paths.
    """
    bad = []
    for f in files:
        base = f.rsplit("/", 1)[-1].lower()
        for forbidden, reason in FORBIDDEN.items():
            if base == forbidden.lower():
                bad.append((f, reason))
    return sorted(bad)


def self_test() -> int:
    """A guard that cannot catch what it exists for is decoration."""
    print("=== self-test ===")
    ok = True
    hits = scan({"INCIDENT_PATTERNS.md", "README.md", "src/main.rs"})
    print(f"  {'OK ' if hits else 'BAD'}  catches an internal doc when present")
    ok &= bool(hits)
    hits = scan({"README.md", "CONTRIBUTING.md", "SECURITY.md", "docs/design.md"})
    print(f"  {'OK ' if not hits else 'BAD'}  lets legitimate public docs through")
    ok &= not hits
    hits = scan({"INCIDENT_PATTERNS.md", "PENDIENTES-SIN-PUBLICAR.md"})
    print(f"  {'OK ' if len(hits) == 2 else 'BAD'}  catches every named file, not just the first")
    ok &= len(hits) == 2
    # Moving it into a subfolder does not make it public documentation.
    hits = scan({"docs/internal/PLAN.md"})
    print(f"  {'OK ' if hits else 'BAD'}  catches it in a subfolder, not only at the top")
    ok &= bool(hits)
    # Windows does not care about case, so neither can this.
    hits = scan({"plan.md"})
    print(f"  {'OK ' if hits else 'BAD'}  catches it with different capitalisation")
    ok &= bool(hits)
    # And a file that merely mentions the name is not the file.
    hits = scan({"docs/why-we-removed-internal-notes.md"})
    print(f"  {'OK ' if not hits else 'BAD'}  does not fire on a doc that just names one")
    ok &= not hits
    print("\n" + ("OK: it catches internal docs and leaves public ones alone." if ok else "BAD: fix it."))
    return 0 if ok else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--self-test"]:
        sys.exit(self_test())
    bad = scan(tracked())
    if bad:
        print("REJECTED: internal documents in a public repository.\n")
        for f, reason in bad:
            print(f"  {f}\n      {reason}")
        print("\nBrisvia's repository is read by strangers. These are for us, not for them.")
        print("Keep them outside any git repository, or in a private one.")
        print("If a rule in one of them matters to outside contributors, extract that rule -- in")
        print("English and sanitised -- into CONTRIBUTING.md or SECURITY.md instead.")
        sys.exit(1)
    print(f"OK: none of the {len(FORBIDDEN)} internal documents are in the public tree.")
