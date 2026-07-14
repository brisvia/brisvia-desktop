#!/usr/bin/env python3
"""Run a test suite and PROVE it actually ran. Exit code zero is not proof.

Why this exists, in one real case from today: the wallet derivation battery (10,000 fresh wallets) had been
red for hours, on every commit, including three versions already published to users. It never ran a single
test -- it died on a build error before starting. Nobody noticed, because nobody was watching a workflow that
had quietly turned red, and because "the tests pass" had become something believed rather than checked.

The failure mode this guards is subtler than a red build: a suite that goes GREEN having executed nothing.
That happens when a filter matches no tests, a rename orphans a module, a feature flag hides them, or someone
adds `|| true`. The command succeeds. The report says ok. Zero tests ran. Every downstream claim of "verified"
is then false, and looks exactly like the truth.

So this asserts the outcome, not the exit code:
  - the suite ran at least `--min` tests (a floor per suite; raise it when you add tests),
  - zero failures,
  - the run is not suspiciously instant (a real suite takes time; an empty one returns immediately).

Usage:
  python tools/run_tests_verified.py --min 29 -- cargo test --manifest-path crates/... --lib
"""
import argparse
import re
import subprocess
import sys
import time


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--min", type=int, required=True, help="minimo de tests que DEBEN ejecutarse")
    ap.add_argument("--name", default="suite", help="nombre para el reporte")
    ap.add_argument("--min-seconds", type=float, default=0.0, help="si termina antes, es sospechoso")
    ap.add_argument("cmd", nargs=argparse.REMAINDER)
    a = ap.parse_args()

    cmd = a.cmd[1:] if a.cmd and a.cmd[0] == "--" else a.cmd
    if not cmd:
        sys.exit("falta el comando a ejecutar (despues de --)")

    print(f"== {a.name}: {' '.join(cmd)}")
    t0 = time.time()
    r = subprocess.run(cmd, capture_output=True, text=True)
    dur = time.time() - t0
    salida = r.stdout + r.stderr
    print(salida[-4000:] if len(salida) > 4000 else salida)

    # cargo test: "test result: ok. 29 passed; 0 failed; 0 ignored; ..."
    corridos = failed = 0
    for m in re.finditer(r"test result: \w+\. (\d+) passed; (\d+) failed", salida):
        corridos += int(m.group(1))
        failed += int(m.group(2))

    fallos = []
    if r.returncode != 0:
        fallos.append(f"el comando fallo (codigo {r.returncode})")
    if failed > 0:
        fallos.append(f"{failed} test(s) fallaron")
    if corridos == 0:
        fallos.append(
            "EJECUTO CERO TESTS. El comando puede haber 'pasado' sin probar nada: un filtro que no matchea, "
            "un modulo renombrado, una feature que los esconde. Verde sin haber probado nada es peor que rojo."
        )
    elif corridos < a.min:
        fallos.append(
            f"ejecuto {corridos} tests pero se esperaban al menos {a.min}. O se perdieron tests, o una suite "
            f"stopped being discovered. If you deleted them on purpose, lower the minimum in the workflow and say why."
        )
    if a.min_seconds and dur < a.min_seconds and not fallos:
        fallos.append(f"termino en {dur:.1f}s, sospechosamente rapido (se esperaban >{a.min_seconds}s)")

    print(f"\n== {a.name}: {corridos} tests ejecutados, {failed} fallidos, en {dur:.1f}s (minimo exigido: {a.min})")
    if fallos:
        print("\nFALLA LA VERIFICACION DE EJECUCION:\n")
        for f in fallos:
            print("  - " + f)
        sys.exit(1)
    print(f"OK: {a.name} corrio de verdad ({corridos} tests, todos en verde).")


if __name__ == "__main__":
    main()
