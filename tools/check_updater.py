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
ESPERADAS = {"windows-x86_64", "darwin-aarch64", "linux-x86_64"}

fallos = []


def head(url):
    try:
        return urllib.request.urlopen(urllib.request.Request(url, method="HEAD"), timeout=45).status
    except urllib.error.HTTPError as e:
        return e.code
    except Exception as e:
        return str(e)


print(f"consultando el manifest como lo hace la app:\n  {MANIFEST}")
try:
    with urllib.request.urlopen(MANIFEST, timeout=45) as r:
        cuerpo = r.read().decode()
    print("  HTTP 200")
except Exception as e:
    print(f"\nFALLA: la app NO puede leer el manifest -> {e}")
    print("Nadie se entera de la actualizacion. Falta subir latest.json al release marcado como 'latest'")
    print("(generarlo con tools/make_latest_json.py <tag>).")
    sys.exit(1)

try:
    d = json.loads(cuerpo)
except json.JSONDecodeError as e:
    sys.exit(f"FALLA: el manifest no es JSON valido -> {e}")

version = d.get("version", "")
print(f"  version que ofrece: {version}")

if len(sys.argv) > 1 and version != sys.argv[1].lstrip("v"):
    fallos.append(f"el manifest ofrece {version} pero se esperaba {sys.argv[1].lstrip('v')}")

plats = d.get("platforms", {})
for falta in ESPERADAS - set(plats):
    fallos.append(f"falta la plataforma {falta}: esa gente se queda sin actualizacion y sin enterarse")

for plat, v in plats.items():
    if not v.get("signature", "").strip():
        fallos.append(f"{plat}: firma vacia (la app rechaza la actualizacion sin firma valida)")
    url = v.get("url", "")
    code = head(url)
    print(f"  {plat:<16} HTTP {code}  {url.split('/')[-1]}")
    if code != 200:
        fallos.append(f"{plat}: el instalador no baja ({url} -> {code})")

if fallos:
    print("\nFALLA EL ACTUALIZADOR:\n")
    for f in fallos:
        print("  - " + f)
    sys.exit(1)
print(f"\nOK: la app ve la {version} y las {len(plats)} plataformas bajan con firma.")
