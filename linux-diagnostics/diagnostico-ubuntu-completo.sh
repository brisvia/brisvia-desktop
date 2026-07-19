#!/usr/bin/env bash
# Brisvia — diagnóstico Ubuntu de UNA SOLA ACCIÓN (Intel/Wayland, 24.04/26.04).
# Baja los instaladores OFICIALES (AppImage + .deb, verifica SHA), prueba abrir la interfaz bajo todas las
# combinaciones, usa un datadir DESCARTABLE (NUNCA toca tu billetera), y SUBE la evidencia a un enlace que
# le pasás a Claude. No instala nada en tu sistema. No pide tus 12 palabras. No necesita sudo.
#
# Correr con UN comando:
#   curl -fsSL https://raw.githubusercontent.com/brisvia/brisvia-desktop/fix/ubuntu-2604-appimage/linux-diagnostics/diagnostico-ubuntu-completo.sh | bash
set -u
SECS=8
AI_SHA=bbc81069bea4c009bfad9a1272f1e5cb23ada419fecf327af0926bdd424b1eec
DEB_SHA=2d13ac45ffce506915d8c94f5f3b99cf8199b2bb49e4a3c9a0aa36253c82f9a2
AI_URL="https://github.com/brisvia/brisvia-desktop/releases/latest/download/Brisvia-Miner-Linux.AppImage"
DEB_URL="https://github.com/brisvia/brisvia-desktop/releases/latest/download/Brisvia.Miner_1.0.8_amd64.deb"

WORK="$(mktemp -d)"; cd "$WORK"
OUT="$WORK/evidencia.txt"; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }

log "=================================================================="
log " Brisvia — diagnóstico Ubuntu (una sola corrida)  $(date -u 2>/dev/null)"
log "=================================================================="
log "Sesión : ${XDG_SESSION_TYPE:-desconocida}"
log "Distro : $( (. /etc/os-release 2>/dev/null; echo "$PRETTY_NAME") )"
log "Kernel : $(uname -r 2>/dev/null)"
log "GLib   : $(pkg-config --modversion glib-2.0 2>/dev/null || ldconfig -p 2>/dev/null | grep -m1 libglib-2.0 | sed 's/.*=> //')"
log "GPU    : $(command -v glxinfo >/dev/null 2>&1 && glxinfo 2>/dev/null | grep -m1 'OpenGL renderer' || echo 'glxinfo no instalado')"
log ""

log ">> Bajando instaladores oficiales (release v1.0.8)..."
curl -fsSL -o app.AppImage "$AI_URL" && chmod +x app.AppImage || log "  (no se pudo bajar el AppImage)"
curl -fsSL -o app.deb "$DEB_URL" || log "  (no se pudo bajar el .deb)"
A=$(sha256sum app.AppImage 2>/dev/null | cut -d' ' -f1); D=$(sha256sum app.deb 2>/dev/null | cut -d' ' -f1)
log "  AppImage SHA: ${A:0:16}… -> $([ "$A" = "$AI_SHA" ] && echo OK || echo DISTINTO)"
log "  .deb     SHA: ${D:0:16}… -> $([ "$D" = "$DEB_SHA" ] && echo OK || echo DISTINTO)"
log ""

verdict(){
  local f="$1"
  grep -qi "EGL_BAD_PARAMETER" "$f" && { echo "FALLA: EGL_BAD_PARAMETER"; return; }
  grep -qi "g_task_set_static_name" "$f" && echo "AVISO: conflicto GLib/GVFS"
  grep -qiE "Could not create default EGL|Aborting|cannot open display|Segmentation fault|undefined symbol" "$f" && { echo "FALLA: no inició los gráficos"; return; }
  echo "OK: parece haber ABIERTO la ventana (sin error fatal en ${SECS}s)"
}
run(){ # etiqueta binario env...
  local name="$1"; local bin="$2"; shift 2
  local lf; lf="$(mktemp)"; local dt; dt="$(mktemp -d)"
  log ">> $name"
  ( env BRISVIA_DATADIR="$dt" "$@" "$bin" ) >"$lf" 2>&1 &
  local pid=$!; sleep "$SECS"; kill "$pid" 2>/dev/null; sleep 1; kill -9 "$pid" 2>/dev/null
  pkill -f "$(basename "$bin")" 2>/dev/null; rm -rf "$dt"
  log "   -> $(verdict "$lf")"
  tail -8 "$lf" | sed 's/^/   | /' | tee -a "$OUT" >/dev/null
  rm -f "$lf"
}

if [ -x app.AppImage ]; then
  run "[AppImage] 1. sin cambios" ./app.AppImage
  run "[AppImage] 2. DMABUF off" ./app.AppImage WEBKIT_DISABLE_DMABUF_RENDERER=1
  run "[AppImage] 3. DMABUF+compositor off (arreglo de la rama)" ./app.AppImage WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1
  run "[AppImage] 4. + X11 (XWayland)" ./app.AppImage GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1
  run "[AppImage] 5. + software" ./app.AppImage LIBGL_ALWAYS_SOFTWARE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1
  log ">> [AppImage] 6. sin GLib empaquetada (usa la del sistema)"
  ( ./app.AppImage --appimage-extract >/dev/null 2>&1 )
  if [ -d squashfs-root ]; then
    find squashfs-root -type f \( -name 'libglib-2.0.so*' -o -name 'libgio-2.0.so*' -o -name 'libgobject-2.0.so*' -o -name 'libgmodule-2.0.so*' \) -delete
    run "[AppImage] 6. sin GLib empaquetada" squashfs-root/AppRun WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1
  fi
fi

log ">> [.deb] extraigo (sin instalar, sin sudo) y corro su binario (usa libs del sistema)"
if dpkg-deb -x app.deb debroot 2>/dev/null; then
  DEBBIN=$(find debroot -type f -name 'brisvia-miner' | head -1)
  [ -n "$DEBBIN" ] && run "[.deb] binario (system libs)" "$DEBBIN" WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 || log "   (no encontré el binario en el .deb)"
  log "   Nota: si el .deb no encuentra sus recursos así, la prueba REAL del .deb es instalarlo (sudo dpkg -i app.deb; brisvia-miner)."
else
  log "   (no se pudo extraer el .deb)"
fi

log ""
log "=================================================================="
log " ANÁLISIS AUTOMÁTICO"
WIN=$(grep -B1 "OK: parece haber ABIERTO" "$OUT" | grep -m1 "^>>" | sed 's/>> //')
if [ -n "$WIN" ]; then
  log " -> ABRIÓ con: $WIN"
  log "    (variante 3 = el arreglo de la rama lo resuelve; .deb o variante 6 = el conflicto de GLib se evita)"
else
  log " -> NINGUNA variante abrió en ${SECS}s. Con la evidencia decidimos el siguiente paso."
fi
log " (Todas las pruebas usaron un datadir DESCARTABLE: NO se tocó la billetera real.)"
log "=================================================================="

echo ""
echo ">> Subiendo la evidencia..."
URL=$(curl -fsSL -F "file=@$OUT" https://0x0.st 2>/dev/null)
[ -z "$URL" ] && URL=$(curl -fsSL --upload-file "$OUT" https://transfer.sh/brisvia-diag.txt 2>/dev/null)
cd /; # dejar copia local por las dudas
cp "$OUT" "$HOME/brisvia-diagnostico-$(hostname 2>/dev/null || echo host).txt" 2>/dev/null
echo "=================================================================="
if [ -n "$URL" ]; then
  echo " LISTO. Pasale ESTE ENLACE a Claude:"
  echo "   $URL"
else
  echo " No pude subir la evidencia (sin internet a los servicios de subida)."
  echo " Está guardada en: $HOME/brisvia-diagnostico-*.txt — mandale ese archivo a Claude."
fi
echo "=================================================================="
rm -rf "$WORK"
