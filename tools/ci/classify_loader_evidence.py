"""Classify the Windows loader diagnostic -- but only after proving the evidence is this run's.

WHY THE IDENTITY GATE EXISTS
----------------------------
Run 29460039297 finished and a stale summary from an EARLIER run said "step 16 failed". Step 16 had
actually passed. Two more minutes and Case A/B/C would have been declared from another run's evidence.

Nothing had been mis-downloaded: `gh run download 29460039297` is scoped server-side and cannot return
another run's files. The evidence was fine; the reading was contaminated. So the gate defends both ends:

  binding    identity comes from the API for one explicit run_id. Never "the newest file", never a
             directory left over from before -- a fresh dir per (run_id, attempt), and it refuses to reuse
             one that already exists.
  stamping   when a manifest carries its own run_id, it must match. Belt and braces against local mixing.

HONEST LIMIT, STATED UP FRONT
-----------------------------
Run 29461779944's manifest does NOT carry an embedded run_id -- the workflow predates this gate, and it
cannot be changed while it runs. For that run identity rests on the API binding alone, which is
authoritative for provenance but proves nothing about local file mixing. The classifier says so out loud
in its report rather than implying an identity check it did not perform. `--require-stamp` makes the
embedded stamp mandatory; future runs will be launched with it.

Usage:
    python classify_loader_evidence.py --run-id 29461779944
    python classify_loader_evidence.py --self-test
"""
import argparse
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import sys

REPO = "brisvia/brisvia-desktop"

# Windows says the loader started a DLL and could not find a procedure in it. It is NOT "a DLL is
# missing" -- that is 0xC0000135. The distinction is the entire investigation.
ENTRYPOINT_NOT_FOUND = -1073741511  # 0xC0000139

APROBADOS = {
    "Invoke-NativeProcess.ps1": "2AEA7817",
    "Test-DiagnosticPreflight.ps1": "A1B39BE3",
}


class EvidenciaRechazada(Exception):
    """The evidence is not provably this run's. Refusing to classify is the correct outcome."""


# The only keys that are a run id. Exact match, nothing else. See leer_sello.
CLAVES_SELLO = ("run_id", "run_attempt_run_id", "github_run_id")


def leer_sello(man):
    """Return the manifest's own run id as a string, or None. Never a lookalike.

    THE FALSE POSITIVE THIS EXISTS TO KILL
    --------------------------------------
    Asked whether the manifest carried a run_id, a substring test answered yes -- because
    'expected_runner' and 'actual_runner' contain the letters "run". The manifest had no run id at all.
    A substring test on a key name is not a field check; it is a coincidence detector. That was the third
    time in one day that a check reported green by looking at the wrong thing.

    So: parse the JSON, address the exact property, check the type, check the value. A key that merely
    resembles one is not one.
    """
    if not isinstance(man, dict):
        raise EvidenciaRechazada(f"the manifest is {type(man).__name__}, not an object")

    for k in CLAVES_SELLO:
        if k not in man:
            continue
        v = man[k]
        # A run id is an integer or the digits of one. Not a bool (bool is an int in Python, and
        # True would sail through an isinstance(v, int) check), not a list, not None, not "".
        if isinstance(v, bool) or not isinstance(v, (int, str)):
            raise EvidenciaRechazada(f"manifest['{k}'] is {type(v).__name__}, not a run id")
        s = str(v).strip()
        if not s.isdigit():
            raise EvidenciaRechazada(f"manifest['{k}'] = {v!r} is not a run id")
        return s
    return None


def _gh(*args):
    r = subprocess.run(["gh", *args], capture_output=True, text=True, encoding="utf-8")
    if r.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args[:2])} failed: {r.stderr.strip()[:200]}")
    return r.stdout


def identidad(run_id):
    """Everything needed to prove which run this is, straight from the API."""
    d = json.loads(_gh("api", f"repos/{REPO}/actions/runs/{run_id}"))
    arts = json.loads(_gh("api", f"repos/{REPO}/actions/runs/{run_id}/artifacts"))["artifacts"]
    return {
        "run_id": str(d["id"]),
        "attempt": d["run_attempt"],
        "url": d["html_url"],
        "workflow_commit": d["head_sha"],
        "branch": d["head_branch"],
        "status": d["status"],
        "conclusion": d["conclusion"],
        "artifacts": [{"name": a["name"], "created_at": a["created_at"],
                       "expired": a["expired"], "id": a["id"]} for a in arts],
    }


def verificar_identidad(ident, run_id, destino):
    """Refuse anything that is not provably this run's, before a single byte is downloaded."""
    if ident["run_id"] != str(run_id):
        raise EvidenciaRechazada(f"the API returned run {ident['run_id']} for {run_id}")
    if ident["status"] != "completed":
        raise EvidenciaRechazada(f"run {run_id} is {ident['status']}: there is nothing final to classify")
    if not ident["artifacts"]:
        raise EvidenciaRechazada(f"run {run_id} produced no artifacts")
    for a in ident["artifacts"]:
        if a["expired"]:
            raise EvidenciaRechazada(f"artifact '{a['name']}' has expired; its bytes are gone")

    # Two artifacts sharing a name is how the wrong one gets read. There is no tie-break that is not a
    # guess, and "the newest" is exactly the guess that started this.
    nombres = [a["name"] for a in ident["artifacts"]]
    dup = {n for n in nombres if nombres.count(n) > 1}
    if dup:
        raise EvidenciaRechazada(f"this run has two artifacts named {', '.join(sorted(dup))}")

    # Never reuse a directory. A leftover from a previous run is exactly how two runs get mixed.
    if destino.exists() and any(destino.iterdir()):
        raise EvidenciaRechazada(
            f"{destino} already has files in it. Downloading into a directory that is not empty is how "
            f"evidence from two runs ends up side by side. Delete it deliberately or pass another --dir.")


def bajar_por_artifact_id(ident, destino):
    """Download each artifact by its own artifact_id, bound to this workflow_run. Hash it immediately.

    Not `gh run download`: that names a run and trusts the result. This addresses each artifact by the id
    the API gave for THIS run, unzips it into a folder of its own, and hashes every byte the moment it
    lands -- before anything else can touch the folder.
    """
    destino.mkdir(parents=True, exist_ok=True)
    registro = []
    for a in ident["artifacts"]:
        # run-<id>-attempt-<n>-<name>: the identity is in the path, so a stray file cannot pass as this
        # run's just by having the right basename.
        sub = destino / f"run-{ident['run_id']}-attempt-{ident['attempt']}-{a['name']}"
        if sub.exists():
            raise EvidenciaRechazada(f"{sub} already exists; refusing to overwrite evidence")
        zip_path = destino / f"{sub.name}.zip"

        r = subprocess.run(
            ["gh", "api", f"repos/{REPO}/actions/artifacts/{a['id']}/zip"],
            capture_output=True)
        if r.returncode != 0 or not r.stdout:
            raise EvidenciaRechazada(
                f"could not download artifact {a['id']} ('{a['name']}'): "
                f"{r.stderr.decode(errors='replace').strip()[:160]}")
        zip_path.write_bytes(r.stdout)

        # Hashed here, immediately, while nothing else has had a chance to touch it.
        sha = hashlib.sha256(r.stdout).hexdigest()
        shutil.unpack_archive(str(zip_path), str(sub), "zip")
        zip_path.unlink()

        archivos = []
        for f in sorted(sub.rglob("*")):
            if f.is_file():
                archivos.append({"path": str(f.relative_to(sub)),
                                 "sha256": hashlib.sha256(f.read_bytes()).hexdigest()})
        registro.append({
            "artifact_id": a["id"], "name": a["name"], "size_reported": a.get("size_in_bytes"),
            "size_downloaded": len(r.stdout), "created_at": a["created_at"],
            "zip_sha256": sha, "dir": str(sub), "files": archivos,
        })
    return registro


def buscar_manifiesto(registro, destino):
    """Exactly one preflight manifest, from a known artifact dir. Not 'the first one found'."""
    hallados = [p for p in destino.rglob("preflight-manifest.json")]
    if not hallados:
        raise EvidenciaRechazada("no preflight-manifest.json among this run's artifacts")
    if len(hallados) > 1:
        raise EvidenciaRechazada(
            f"{len(hallados)} preflight manifests: {', '.join(str(p) for p in hallados)}. "
            f"There is no way to pick one that is not a guess.")
    return json.loads(hallados[0].read_text(encoding="utf-8-sig"))


def cruzar_manifiesto(man, ident, exigir_sello):
    notas = []
    if man is None:
        raise EvidenciaRechazada("no preflight-manifest.json among the artifacts")

    sello = leer_sello(man)
    if sello is None:
        if exigir_sello:
            raise EvidenciaRechazada(
                "the manifest carries no run_id. Identity would rest on the API binding alone, which "
                "proves provenance but not that these local files are unmixed. Refusing under "
                "--require-stamp.")
        notas.append("NO EMBEDDED run_id: identity rests on the API binding alone. This run predates "
                     "the stamp. Local mixing is NOT ruled out by the manifest itself.")
    elif str(sello) != ident["run_id"]:
        raise EvidenciaRechazada(
            f"the manifest says run {sello}; the API says {ident['run_id']}. These are different runs.")
    else:
        notas.append(f"embedded run_id {sello} matches the API")

    if man.get("sha") != ident["workflow_commit"]:
        raise EvidenciaRechazada(
            f"the manifest was built from {str(man.get('sha'))[:8]} but the run is "
            f"{ident['workflow_commit'][:8]}: two different rulers")
    notas.append(f"workflow commit {ident['workflow_commit'][:8]} matches the manifest")

    if man.get("status") != "PASS":
        raise EvidenciaRechazada(f"the preflight did not pass: {man.get('status')}")

    for nombre, esperado in APROBADOS.items():
        real = man.get("script_hashes", {}).get(nombre, "")
        if not str(real).upper().startswith(esperado):
            raise EvidenciaRechazada(
                f"{nombre} hashes {str(real)[:8] or '(absent)'}, approved is {esperado}: the tool changed")
    notas.append(f"{len(APROBADOS)} approved script hashes match")
    return notas


def leer_mitades(run_id):
    """The two exit codes, from the log of this run and no other."""
    log = _gh("run", "view", str(run_id), "--repo", REPO, "--log")
    mitades = {}
    for etiqueta in ("BASELINE", "CANDIDATE"):
        # The helper prints "exit=N hex=0xXXXXXXXX" inside the "does it load?" step and nowhere else.
        patron = re.compile(rf"{etiqueta} - does it load.*?exit=(-?\d+) hex=(0x[0-9A-Fa-f]+)", re.S)
        m = patron.search(log)
        mitades[etiqueta] = (int(m.group(1)), m.group(2)) if m else (None, None)
    return mitades


def clasificar(mitades):
    b, c = mitades["BASELINE"], mitades["CANDIDATE"]
    if b[0] is None or c[0] is None:
        falta = "BASELINE" if b[0] is None else "CANDIDATE"
        return "UNCLASSIFIED", f"the {falta} half never reported an exit code"
    if b[0] != 0:
        return "B", (f"the baseline does not load either (exit {b[0]} / {b[1]}). The diagnostic is still "
                     f"invalid: the environment differs from the real job. Fix the tool, not the product.")
    if c[0] == ENTRYPOINT_NOT_FOUND:
        return "A", (f"baseline loads (exit 0), candidate dies with {c[1]} = STATUS_ENTRYPOINT_NOT_FOUND. "
                     f"Windows started a DLL and could not find a procedure in it. The regression is real "
                     f"and reproduced. Next: /DEPENDENTS, /IMPORTS, the concrete DLL, /EXPORTS.")
    if c[0] == 0:
        return "C", ("both halves load. The rc5-rc6 change set does not reproduce it here. Compare against "
                     "the real failing job 29448577901 before attributing anything.")
    return "UNCLASSIFIED", (f"the candidate exits {c[0]} / {c[1]}, which is neither 0 nor 0xC0000139. "
                              f"This is a different failure and none of A, B or C describes it.")


# -------------------------------------------------------------------------------------------------

def self_test():
    """Mix two runs' evidence on purpose and prove the classifier refuses."""
    fallos = 0

    def check(nombre, fn, espera_rechazo=True):
        nonlocal fallos
        try:
            fn()
            if espera_rechazo:
                print(f"  FAIL  {nombre}: it accepted evidence it should have refused")
                fallos += 1
            else:
                print(f"  PASS  {nombre}")
        except EvidenciaRechazada as e:
            if espera_rechazo:
                print(f"  PASS  {nombre}")
                print(f"          -> {str(e)[:95]}")
            else:
                print(f"  FAIL  {nombre}: refused valid evidence: {e}")
                fallos += 1

    real = {"run_id": "29461779944", "attempt": 1, "workflow_commit": "4ba04c59" + "2" * 32}
    bueno = {"status": "PASS", "sha": real["workflow_commit"], "run_id": "29461779944",
             "script_hashes": {k: v + "00" for k, v in APROBADOS.items()}}

    # THE one that matters: a manifest from a different run, with the right filename.
    otro = dict(bueno, run_id="29460039297")
    check("refuses-a-manifest-from-another-run", lambda: cruzar_manifiesto(otro, real, False))

    # The false positive, reproduced exactly. This is the real 29461779944 manifest's key set: no run id
    # anywhere, but two keys containing the letters "run". A substring test said "SI, lleva run_id".
    real_29461779944 = {"status": "PASS", "generated_utc": "2026-07-15T23:57:09Z",
                        "expected_runner": "windows-2022", "actual_runner": "windows-2022",
                        "sha": real["workflow_commit"], "ref": "refs/heads/fix/diagnostic-parse-guard",
                        "powershell": "7.4.6", "script_hashes": {}, "checks": [], "checks_total": 14,
                        "checks_failed": 0}
    if leer_sello(real_29461779944) is not None:
        print("  FAIL  expected_runner/actual_runner were mistaken for a run id -- the exact false "
              "positive this exists to kill")
        fallos += 1
    else:
        print("  PASS  refuses-expected_runner-as-a-run-id")

    for valor, porque in ((True, "a bool"), ([29461779944], "a list"), ("", "empty"),
                          ("no-soy-un-id", "not digits"), (None, "null")):
        try:
            leer_sello({"run_id": valor})
            print(f"  FAIL  accepted {porque} as a run id")
            fallos += 1
        except EvidenciaRechazada:
            pass
    print("  PASS  refuses-a-run_id-that-is-a-bool-list-empty-nondigit-or-null")

    if leer_sello({"run_id": 29461779944}) != "29461779944":
        print("  FAIL  an integer run_id was not read")
        fallos += 1
    else:
        print("  PASS  reads-an-integer-run_id-as-well-as-a-string")

    # Same run, but built from a different commit: two different rulers.
    check("refuses-another-commit", lambda: cruzar_manifiesto(dict(bueno, sha="d" * 40), real, False))

    # The tool itself changed between preflight and use.
    check("refuses-changed-tool", lambda: cruzar_manifiesto(
        dict(bueno, script_hashes={"Invoke-NativeProcess.ps1": "DEADBEEF",
                                   "Test-DiagnosticPreflight.ps1": "A1B39BE300"}), real, False))

    check("refuses-a-failed-preflight", lambda: cruzar_manifiesto(dict(bueno, status="FAIL"), real, False))

    sin = {k: v for k, v in bueno.items() if k != "run_id"}
    check("refuses-an-unstamped-manifest-under-require-stamp",
          lambda: cruzar_manifiesto(sin, real, True))
    check("allows-an-unstamped-manifest-otherwise-but-says-so",
          lambda: cruzar_manifiesto(sin, real, False), espera_rechazo=False)
    notas = cruzar_manifiesto(sin, real, False)
    if not any("NO EMBEDDED" in n for n in notas):
        print("  FAIL  an unstamped manifest passed silently; the report would imply a check that did "
              "not happen")
        fallos += 1
    else:
        print("  PASS  an-unstamped-manifest-is-flagged-loudly")

    check("accepts-matching-evidence", lambda: cruzar_manifiesto(bueno, real, False),
          espera_rechazo=False)

    # The classifier must not invent a case from a missing half.
    for nombre, mitades, espera in (
            ("no-case-when-a-half-is-missing", {"BASELINE": (0, "0x0"), "CANDIDATE": (None, None)},
             "UNCLASSIFIED"),
            ("case-A-is-the-real-signature", {"BASELINE": (0, "0x00000000"),
                                              "CANDIDATE": (ENTRYPOINT_NOT_FOUND, "0xC0000139")}, "A"),
            ("case-B-when-the-baseline-fails", {"BASELINE": (ENTRYPOINT_NOT_FOUND, "0xC0000139"),
                                                "CANDIDATE": (ENTRYPOINT_NOT_FOUND, "0xC0000139")}, "B"),
            ("case-C-when-both-load", {"BASELINE": (0, "0x0"), "CANDIDATE": (0, "0x0")}, "C"),
            ("no-case-for-an-unrelated-exit", {"BASELINE": (0, "0x0"), "CANDIDATE": (101, "0x65")},
             "UNCLASSIFIED")):
        caso, _ = clasificar(mitades)
        if caso != espera:
            print(f"  FAIL  {nombre}: said {caso}, expected {espera}")
            fallos += 1
        else:
            print(f"  PASS  {nombre}")

    print()
    if fallos:
        print(f"SELF-TEST FAILED ({fallos})")
        return 1
    print("self-test OK: the gate refuses mixed evidence and the classifier invents nothing")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--run-id")
    ap.add_argument("--dir")
    ap.add_argument("--require-stamp", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()

    if a.self_test:
        return self_test()
    if not a.run_id:
        return ap.error("--run-id is required")

    destino = pathlib.Path(a.dir) if a.dir else pathlib.Path(
        f"C:/dev/brisvia-miner/evidence/run-{a.run_id}")

    ident = identidad(a.run_id)
    print("=" * 78)
    print("EVIDENCE IDENTITY")
    print("=" * 78)
    for k in ("run_id", "attempt", "url", "workflow_commit", "branch", "status", "conclusion"):
        print(f"  {k:<16} {ident[k]}")
    print()

    try:
        verificar_identidad(ident, a.run_id, destino)
        registro = bajar_por_artifact_id(ident, destino)
        for r in registro:
            print(f"  artifact_id      {r['artifact_id']}  '{r['name']}'")
            print(f"    created        {r['created_at']}")
            print(f"    size           {r['size_reported']} reported / {r['size_downloaded']} downloaded")
            print(f"    zip sha256     {r['zip_sha256']}")
            for f in r["files"]:
                print(f"    {f['sha256'][:16]}...  {f['path']}")
        print()
        man = buscar_manifiesto(registro, destino)
        notas = cruzar_manifiesto(man, ident, a.require_stamp)
    except EvidenciaRechazada as e:
        print("=" * 78)
        print("EVIDENCE REJECTED -- no Case A, B, C or UNCLASSIFIED is emitted")
        print("=" * 78)
        print(f"  {e}")
        return 2

    for n in notas:
        print(f"  {n}")

    # Said out loud, never implied. API-bound proves where the bytes came from; it does not prove the
    # local copy was not mixed with another run's. Only an embedded stamp proves that.
    sellado = leer_sello(man) is not None
    print()
    print(f"  IDENTITY LEVEL: {'API-BOUND + EMBEDDED RUN STAMP' if sellado else 'API-BOUND, NO EMBEDDED RUN STAMP'}")
    if not sellado:
        print("    -> enough to choose the next test. NOT enough to change the product.")
    print()

    mitades = leer_mitades(a.run_id)
    print("=" * 78)
    print("THE TWO HALVES")
    print("=" * 78)
    for k, (code, hexa) in mitades.items():
        print(f"  {k:<10} exit={code if code is not None else 'not reported'}  hex={hexa or '-'}")
    print()

    caso, razon = clasificar(mitades)
    print("=" * 78)
    print(f"CASE {caso}" + ("  (provisional)" if not sellado else ""))
    print("=" * 78)
    print(f"  {razon}")

    if caso == "A" and not sellado:
        print()
        print("  DOES NOT AUTHORIZE A PRODUCT FIX ON ITS OWN.")
        print("  Next: design the three ablations from this evidence and run them stamped -- run_id,")
        print("  commit, source SHA, tool hashes and manifest inside each artifact. The product changes")
        print("  only after a stamped ablation is conclusive.")

    (destino / "classification.json").write_text(json.dumps({
        "case": caso, "reason": razon, "provisional": not sellado,
        "identity_level": "API-BOUND + EMBEDDED RUN STAMP" if sellado else
                          "API-BOUND, NO EMBEDDED RUN STAMP",
        "identity": ident, "artifacts": registro,
        "halves": {k: {"exit": v[0], "hex": v[1]} for k, v in mitades.items()},
    }, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
