#!/usr/bin/env python3
"""Candado: los SHA-256 que publica brisvia.com deben ser los del archivo que la gente REALMENTE baja.

Por que existe: la web pone un SHA-256 al lado de cada boton de descarga y le pide a la gente que verifique
el archivo antes de ejecutarlo. Los botones apuntan a /releases/latest/ (siempre la ultima version), pero los
hashes se escriben a mano. Al publicar la 1.0.5 quedaron los de una version vieja, LOS SEIS:

    la web decia:  e99ff6b0...      el archivo que baja la gente:  6025b4af...

Alguien baja Brisvia, hace la verificacion que la propia web le pide, no coincide, y concluye que le dieron
un archivo adulterado. El mecanismo que existe para dar CONFIANZA hacia lo contrario -- y se rompia solo en
cada version, sin que nada avisara.

Lee la web PUBLICA y baja los instaladores desde la URL PUBLICA: lo que importa es lo que recibe la gente,
no lo que quedo en un disco de desarrollo.

Uso:  python tools/check_web_hashes.py
"""
import hashlib
import re
import sys
import urllib.request

WEB = "https://brisvia.com"
BASE = "https://github.com/brisvia/brisvia-desktop/releases/latest/download"
PAGES = ["descargas.html", "downloads.html"]  # Spanish + English publish the same hashes
UA = {"User-Agent": "Mozilla/5.0"}

ARCHIVOS = {
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
    print("Comparando los hashes que publica " + WEB + " con los instaladores reales.\n")
    reales = {}
    for hid, nombre in ARCHIVOS.items():
        try:
            reales[hid] = sha256_url(BASE + "/" + nombre)
            print("  %-32s %s..." % (nombre, reales[hid][:24]))
        except Exception as e:
            print("  FALLA bajando %s: %s" % (nombre, e))
            return 1

    fallos = []
    for pag in PAGINAS:
        try:
            req = urllib.request.Request(WEB + "/" + pag, headers=UA)
            s = urllib.request.urlopen(req, timeout=60).read().decode("utf-8", "ignore")
        except Exception as e:
            fallos.append("%s no responde: %s" % (pag, e))
            print("\n  %s: NO RESPONDE" % pag)
            continue
        print("\n%s:" % pag)
        for hid, real in reales.items():
            m = re.search(r'id="' + hid + r'">([a-f0-9]{64})<', s)
            if not m:
                fallos.append("%s: no publica el hash %s" % (pag, hid))
                print("  %s: NO ESTA PUBLICADO" % hid)
                continue
            if m.group(1) == real:
                print("  %s: OK" % hid)
            else:
                fallos.append("%s/%s: publica %s... y el archivo es %s..."
                              % (pag, hid, m.group(1)[:16], real[:16]))
                print("  %s: MAL  (publica %s... / el archivo es %s...)" % (hid, m.group(1)[:16], real[:16]))

    print()
    if fallos:
        print("FALLA: la web publica hashes que NO son los del archivo que baja la gente.\n")
        for f in fallos:
            print("  - " + f)
        print("\nQuien verifique la descarga -- como la web le pide -- va a creer que le dieron un archivo")
        print("adulterado. Arreglar con: python tools/actualizar_hashes_web.py --aplicar (en cripto-pow)")
        print("y subir la web al servidor.")
        return 1
    print("OK: los hashes publicados coinciden con lo que baja la gente.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
