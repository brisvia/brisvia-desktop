#!/usr/bin/env python3
"""Fail if an internal working document reaches the public repository.

WHY
---
Brisvia is a public coin. Its repository is read by strangers, and everything in it is a statement to
them. Two files were in there that had no business being: CLAUDE.md, the working instructions for the
agent, and INCIDENT_PATTERNS.md, the team's incident log -- real mistakes, procedures, internal
patterns. Neither is documentation anyone outside needs. Nothing in the product read them.

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
PROHIBIDOS = {
    "CLAUDE.md": "working instructions for the agent; not documentation for users or contributors",
    "INCIDENT_PATTERNS.md": "internal incident log: mistakes, procedures and team patterns",
    "PENDIENTES-SIN-PUBLICAR.md": "internal list of unreleased work; its own name says it is not public",
    "MEJORAS-PENDIENTES.md": "internal backlog",
    "LOGROS-50-DISENO.md": "internal design notes",
    "PLAN.md": "internal planning",
}

# Where the private copies live. Named here so anyone reading this failure knows the work was not lost.
PRIVADO = r"C:\dev\brisvia-ops-privado"


def tracked() -> set:
    r = subprocess.run(["git", "ls-files"], capture_output=True, text=True)
    return {l.replace("\\", "/") for l in r.stdout.splitlines() if l}


def revisar(archivos: set) -> list:
    return sorted((f, PROHIBIDOS[f]) for f in PROHIBIDOS if f in archivos)


def self_test() -> int:
    """A guard that cannot catch what it exists for is decoration."""
    print("=== self-test ===")
    ok = True
    hits = revisar({"CLAUDE.md", "README.md", "src/main.rs"})
    print(f"  {'OK ' if hits else 'BAD'}  catches CLAUDE.md when present")
    ok &= bool(hits)
    hits = revisar({"README.md", "CONTRIBUTING.md", "SECURITY.md", "docs/design.md"})
    print(f"  {'OK ' if not hits else 'BAD'}  lets legitimate public docs through")
    ok &= not hits
    hits = revisar({"INCIDENT_PATTERNS.md", "PENDIENTES-SIN-PUBLICAR.md"})
    print(f"  {'OK ' if len(hits) == 2 else 'BAD'}  catches every named file, not just the first")
    ok &= len(hits) == 2
    print("\n" + ("OK: it catches internal docs and leaves public ones alone." if ok else "BAD: fix it."))
    return 0 if ok else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--self-test"]:
        sys.exit(self_test())
    malos = revisar(tracked())
    if malos:
        print("REJECTED: internal documents in a public repository.\n")
        for f, por in malos:
            print(f"  {f}\n      {por}")
        print(f"\nBrisvia's repository is read by strangers. These are for us, not for them.")
        print(f"Keep them in {PRIVADO}, outside any git repo, or in a PRIVATE one.")
        print("If a rule in one of them matters to outside contributors, extract that rule -- in")
        print("English and sanitised -- into CONTRIBUTING.md or SECURITY.md instead.")
        sys.exit(1)
    print(f"OK: none of the {len(PROHIBIDOS)} internal documents are in the public tree.")
