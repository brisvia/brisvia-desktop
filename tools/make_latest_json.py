#!/usr/bin/env python3
"""Build latest.json -- the file every installed copy of Brisvia reads to learn there is a new version.

Why this exists: this file was being written BY HAND after each release. On 1.0.3 it was forgotten, the
release was published as "latest", and the updater broke for everyone at once: the app asks for
releases/latest/download/latest.json, that now pointed at a release without one, and got a 404. Nobody would
have been offered the update, and nothing would have looked wrong from the outside.

It reads the signatures from the release itself (never from a local folder: what is published is the truth),
and REFUSES to write a manifest with a platform missing -- a partial manifest silently strands whoever runs
that platform.

Usage:  python tools/make_latest_json.py <tag>        (needs gh authenticated)
"""
import json
import os
import subprocess
import sys
import tempfile

REPO = "brisvia/brisvia-desktop"

# platform -> (signature file, artifact the updater downloads). The version is filled in from the tag.
PLATAFORMAS = {
    "windows-x86_64": ("Brisvia.Miner_{v}_x64-setup.exe.sig", "Brisvia.Miner_{v}_x64-setup.exe"),
    "darwin-aarch64": ("Brisvia.Miner.app.tar.gz.sig", "Brisvia.Miner.app.tar.gz"),
    "linux-x86_64": ("Brisvia.Miner_{v}_amd64.AppImage.sig", "Brisvia.Miner_{v}_amd64.AppImage"),
}


def gh(*args):
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"fallo `gh {' '.join(args)}`:\n{r.stderr}")
    return r.stdout.strip()


def main():
    if len(sys.argv) < 2:
        sys.exit("uso: make_latest_json.py <tag>   (ej: v1.0.3)")
    tag = sys.argv[1]
    version = tag.lstrip("v")

    pub = gh("release", "view", tag, "--repo", REPO, "--json", "publishedAt", "--jq", ".publishedAt")
    notes = gh("release", "view", tag, "--repo", REPO, "--json", "name", "--jq", ".name")

    tmp = tempfile.mkdtemp(prefix="brisvia-sig-")
    gh("release", "download", tag, "--repo", REPO, "--pattern", "*.sig", "--dir", tmp, "--clobber")

    manifest = {"version": version, "notes": notes, "pub_date": pub, "platforms": {}}
    faltan = []
    for plat, (sig_tpl, art_tpl) in PLATAFORMAS.items():
        sig_name = sig_tpl.format(v=version)
        path = os.path.join(tmp, sig_name)
        if not os.path.exists(path):
            faltan.append(f"{plat}: falta {sig_name}")
            continue
        firma = open(path).read().strip()
        if not firma:
            faltan.append(f"{plat}: {sig_name} esta vacio")
            continue
        manifest["platforms"][plat] = {
            "signature": firma,
            "url": f"https://github.com/{REPO}/releases/download/{tag}/{art_tpl.format(v=version)}",
        }

    # Better no manifest than a half one: a platform missing here means those users are silently stranded on
    # an old version, with nothing looking broken.
    if faltan:
        print("NO escribo el manifest: falta firma de alguna plataforma.", file=sys.stderr)
        for f in faltan:
            print("  - " + f, file=sys.stderr)
        print("\nCorre los 3 builds (Windows/Linux/macOS) y volve a intentar.", file=sys.stderr)
        sys.exit(1)

    out = os.path.join(tmp, "latest.json")
    with open(out, "w") as f:
        f.write(json.dumps(manifest, indent=2) + "\n")
    print(f"manifest de {version} con las {len(manifest['platforms'])} plataformas:")
    for plat, v in manifest["platforms"].items():
        print(f"  {plat:<16} firma {len(v['signature'])} chars -> {v['url'].split('/')[-1]}")
    print(out)
    return out


if __name__ == "__main__":
    main()
