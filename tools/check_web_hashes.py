#!/usr/bin/env python3
"""Lock: the SHA-256 hashes brisvia.com publishes must be the ones of the file people ACTUALLY download.

Why it exists: the site shows a SHA-256 next to each download button and asks people to verify the file
before running it. The buttons point to /releases/latest/ (always the latest version), but the hashes are
written by hand. When 1.0.5 was published, the ones from an old version were left in place, ALL SIX:

    the site said:  e99ff6b0...      the file people download:  6025b4af...

Someone downloads Brisvia, does the verification the site itself asks for, it does not match, and concludes
they were handed a tampered file. The mechanism that exists to give TRUST does the opposite -- and it broke
on its own on every version, with nothing warning about it.

It reads the PUBLIC site and downloads the installers from the PUBLIC URL: what matters is what people
receive, not what was left on a development disk.

Usage:  python tools/check_web_hashes.py
"""
import hashlib
import re
import sys
import urllib.request

WEB = "https://brisvia.com"
BASE = "https://github.com/brisvia/brisvia-desktop/releases/latest/download"
PAGES = ["descargas.html", "downloads.html"]  # Spanish + English publish the same hashes
UA = {"User-Agent": "Mozilla/5.0"}

FILES = {
    "hash-win": "Brisvia-Miner-Windows.exe",
    "hash-mac": "Brisvia-Miner-macOS.dmg",
    "hash-linux": "Brisvia-Miner-Linux.AppImage",
}


def sha256_url(url):
    h = hashlib.sha256()
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=600) as r:
        while True:
            b = r.read(1 << 20)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def main():
    print("Comparing the hashes " + WEB + " publishes against the real installers.\n")
    real_hashes = {}
    for hid, name in FILES.items():
        try:
            real_hashes[hid] = sha256_url(BASE + "/" + name)
            print("  %-32s %s..." % (name, real_hashes[hid][:24]))
        except Exception as e:
            print("  FAIL downloading %s: %s" % (name, e))
            return 1

    failures = []
    for page in PAGES:
        try:
            req = urllib.request.Request(WEB + "/" + page, headers=UA)
            s = urllib.request.urlopen(req, timeout=60).read().decode("utf-8", "ignore")
        except Exception as e:
            failures.append("%s does not respond: %s" % (page, e))
            print("\n  %s: NO RESPONSE" % page)
            continue
        print("\n%s:" % page)
        for hid, real in real_hashes.items():
            m = re.search(r'id="' + hid + r'">([a-f0-9]{64})<', s)
            if not m:
                failures.append("%s: does not publish the hash %s" % (page, hid))
                print("  %s: NOT PUBLISHED" % hid)
                continue
            if m.group(1) == real:
                print("  %s: OK" % hid)
            else:
                failures.append("%s/%s: publishes %s... and the file is %s..."
                              % (page, hid, m.group(1)[:16], real[:16]))
                print("  %s: BAD  (publishes %s... / the file is %s...)" % (hid, m.group(1)[:16], real[:16]))

    print()
    if failures:
        print("FAIL: the site publishes hashes that are NOT those of the file people download.\n")
        for f in failures:
            print("  - " + f)
        print("\nAnyone who verifies the download -- as the site asks them to -- will believe they were handed a")
        print("tampered file. Fix with: python tools/actualizar_hashes_web.py --aplicar (in cripto-pow)")
        print("and upload the site to the server.")
        return 1
    print("OK: the published hashes match what people download.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
