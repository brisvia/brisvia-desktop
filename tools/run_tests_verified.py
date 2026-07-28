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
    ap.add_argument("--min", type=int, required=True, help="minimum number of tests that MUST run")
    ap.add_argument("--name", default="suite", help="name for the report")
    ap.add_argument("--min-seconds", type=float, default=0.0, help="if it finishes sooner, it is suspicious")
    ap.add_argument("cmd", nargs=argparse.REMAINDER)
    a = ap.parse_args()

    cmd = a.cmd[1:] if a.cmd and a.cmd[0] == "--" else a.cmd
    if not cmd:
        sys.exit("missing the command to run (after --)")

    print(f"== {a.name}: {' '.join(cmd)}")
    t0 = time.time()
    r = subprocess.run(cmd, capture_output=True, text=True)
    dur = time.time() - t0
    output = r.stdout + r.stderr
    print(output[-4000:] if len(output) > 4000 else output)

    # cargo test: "test result: ok. 29 passed; 0 failed; 0 ignored; ..."
    ran = failed = 0
    for m in re.finditer(r"test result: \w+\. (\d+) passed; (\d+) failed", output):
        ran += int(m.group(1))
        failed += int(m.group(2))

    failures = []
    if r.returncode != 0:
        failures.append(f"the command failed (code {r.returncode})")
    if failed > 0:
        failures.append(f"{failed} test(s) failed")
    if ran == 0:
        failures.append(
            "RAN ZERO TESTS. The command may have 'passed' without testing anything: a filter that matches none, "
            "a renamed module, a feature that hides them. Green without testing anything is worse than red."
        )
    elif ran < a.min:
        failures.append(
            f"ran {ran} tests but at least {a.min} were expected. Either tests were lost, or a suite "
            f"stopped being discovered. If you deleted them on purpose, lower the minimum in the workflow and say why."
        )
    if a.min_seconds and dur < a.min_seconds and not failures:
        failures.append(f"finished in {dur:.1f}s, suspiciously fast (expected >{a.min_seconds}s)")

    print(f"\n== {a.name}: {ran} tests run, {failed} failed, in {dur:.1f}s (minimum required: {a.min})")
    if failures:
        print("\nEXECUTION VERIFICATION FAILED:\n")
        for f in failures:
            print("  - " + f)
        sys.exit(1)
    print(f"OK: {a.name} really ran ({ran} tests, all green).")


if __name__ == "__main__":
    main()
