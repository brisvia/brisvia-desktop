#!/usr/bin/env bash
# CANDIDATE fix for cause (1): repack an existing Brisvia .AppImage WITHOUT the bundled GLib family, so it uses
# the host's GLib/GVFS consistently and the "undefined symbol: g_task_set_static_name" clash disappears on
# Ubuntu 24.04/26.04.
#
# Why this is safe reasoning (NOT blind removal): libglib-2.0 / libgio-2.0 / libgobject-2.0 / libgmodule-2.0
# are on the official AppImage excludelist precisely because they must match the host's GIO/GVFS modules. The
# app is built on Ubuntu 22.04 (GLib 2.72); every target host has GLib >= 2.72, which is ABI-forward-compatible,
# so the bundled WebKitGTK/GTK resolve all the symbols they need against the host GLib. Removing the bundled
# copies makes the whole GLib/GTK/WebKitGTK set come from ONE place (the host) instead of a split old/new mix.
#
# STATUS: CANDIDATE. Must be validated on real 22.04 / 24.04 / 26.04 before it ships. It is deliberately NOT
# wired into the release workflow (build-linux.yml) so it can never replace a published build without a human.
#
# Usage:  ./repack-appimage-without-glib.sh  input.AppImage  output.AppImage
set -euo pipefail
IN="${1:?uso: repack-appimage-without-glib.sh <entrada.AppImage> <salida.AppImage>}"
OUT="${2:?falta la ruta de salida}"
[ -f "$IN" ] || { echo "no existe: $IN"; exit 2; }
chmod +x "$IN"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
echo ">> extrayendo $IN"
( cd "$WORK" && "$OLDPWD/$IN" --appimage-extract >/dev/null )
AD="$WORK/squashfs-root"
[ -d "$AD" ] || { echo "no se pudo extraer"; exit 1; }

echo ">> quitando la familia GLib empaquetada (se tomara del host):"
find "$AD" -type f \( -name 'libglib-2.0.so*' -o -name 'libgio-2.0.so*' \
     -o -name 'libgobject-2.0.so*' -o -name 'libgmodule-2.0.so*' \) -print -delete | sed 's/^/   /'

# appimagetool para reempaquetar (se descarga si no esta).
TOOL="$(command -v appimagetool || true)"
if [ -z "$TOOL" ]; then
  echo ">> descargando appimagetool"
  TOOL="$WORK/appimagetool"
  curl -fsSL -o "$TOOL" \
    https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x "$TOOL"
fi

echo ">> reempaquetando -> $OUT"
ARCH=x86_64 "$TOOL" "$AD" "$OUT" >/dev/null 2>&1 || ARCH=x86_64 "$TOOL" --appimage-extract-and-run "$AD" "$OUT"
chmod +x "$OUT"
echo ">> listo. SHA-256: $(sha256sum "$OUT" | cut -d' ' -f1)"
echo ">> PROBAR en 22.04/24.04/26.04 antes de considerarlo valido (ver README.md)."
