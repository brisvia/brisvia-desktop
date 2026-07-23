#!/usr/bin/env python3
"""Check the updater the way an installed copy of Brisvia sees it -- from outside, over the network.

Why this exists: publishing 1.0.3 broke the updater for everyone and nothing looked wrong. The release page
was perfect, every installer downloaded, the API reported the manifest as "uploaded". But the one URL the app
actually asks for returned 404, so nobody would ever have been offered the update. Reading GitHub's own
metadata is not enough: the only proof is fetching what the app fetches.

It fails when:
  1. The manifest URL the app polls does not return 200 (this is what broke).
  2. The version it announces is not the one expected.
  3. A platform is missing (those users are silently stranded).
  4. Any signature is empty, or any installer URL does not download.

Usage:  python tools/check_updater.py [expected version]     e.g. check_updater.py 1.0.3
"""
import json
import sys
import urllib.error
import urllib.request

# The exact URL baked into tauri.conf.json -- what every installed copy polls.
MANIFEST = "https://github.com/brisvia/brisvia-desktop/releases/latest/download/latest.json"
EXPECTED = {"windows-x86_64", "darwin-aarch64", "linux-x86_64"}

failures = []


def head(url):
    try:
        return urllib.request.urlopen(urllib.request.Request(url, method="HEAD"), timeout=45).status
    except urllib.error.HTTPError as e:
        return e.code
    except Exception as e:
        return str(e)


print(f"querying the manifest the way the app does:\n  {MANIFEST}")
try:
    with urllib.request.urlopen(MANIFEST, timeout=45) as r:
        body = r.read().decode()
    print("  HTTP 200")
except Exception as e:
    print(f"\nFAIL: the app CANNOT read the manifest -> {e}")
    print("Nobody hears about the update. latest.json still needs to be uploaded to the release marked 'latest'")
    print("(generate it with tools/make_latest_json.py <tag>).")
    sys.exit(1)

try:
    d = json.loads(body)
except json.JSONDecodeError as e:
    sys.exit(f"FAIL: the manifest is not valid JSON -> {e}")

version = d.get("version", "")
print(f"  version it offers: {version}")

if len(sys.argv) > 1 and version != sys.argv[1].lstrip("v"):
    failures.append(f"the manifest offers {version} but {sys.argv[1].lstrip('v')} was expected")

plats = d.get("platforms", {})
for missing in EXPECTED - set(plats):
    failures.append(f"platform {missing} is missing: those users are stranded with no update and no notice")

for plat, v in plats.items():
    if not v.get("signature", "").strip():
        failures.append(f"{plat}: empty signature (the app rejects the update without a valid signature)")
    url = v.get("url", "")
    code = head(url)
    print(f"  {plat:<16} HTTP {code}  {url.split('/')[-1]}")
    if code != 200:
        failures.append(f"{plat}: the installer does not download ({url} -> {code})")

if failures:
    print("\nUPDATER FAILED:\n")
    for f in failures:
        print("  - " + f)
    sys.exit(1)
print(f"\nOK: the app sees {version} and all {len(plats)} platforms download with a signature.")
