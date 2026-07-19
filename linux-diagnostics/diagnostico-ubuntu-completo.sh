#!/usr/bin/env bash
# Brisvia — diagnóstico Ubuntu (Intel/Wayland, 24.04/26.04). Correr EN LA THINKPAD (Ubuntu), no en Windows.
# Baja los instaladores OFICIALES (AppImage + .deb, verifica SHA), prueba abrir la interfaz bajo varias
# combinaciones (cada intento con un datadir DESCARTABLE: NUNCA toca tu billetera), y GUARDA la evidencia en
# un archivo local. La subida a un enlace es OPCIONAL y te la pregunta al final. No instala nada. No pide tu semilla.
set -u
SECS=8
AI_SHA=bbc81069bea4c009bfad9a1272f1e5cb23ada419fecf327af0926bdd424b1eec
DEB_SHA=2d13ac45ffce506915d8c94f5f3b99cf8199b2bb49e4a3c9a0aa36253c82f9a2
AI_URL="https://github.com/brisvia/brisvia-desktop/releases/latest/download/Brisvia-Miner-Linux.AppImage"
DEB_URL="https://github.com/brisvia/brisvia-desktop/releases/latest/download/Brisvia.Miner_1.0.8_amd64.deb"

WORK="$(mktemp -d)"; cd "$WORK"
DEST="$HOME/brisvia-diagnostico-$(hostname 2>/dev/null || echo host).txt"
OUT="$WORK/evidencia.txt"; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }

log "=================================================================="
log " Brisvia — diagnóstico Ubuntu  $(date -u 2>/dev/null)"
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
  echo "SIN ERROR FATAL en ${SECS}s (proceso vivo; confirmá VOS si se vio la ventana)"
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
  run "[AppImage] 5. + software (llvmpipe)" ./app.AppImage LIBGL_ALWAYS_SOFTWARE=1 MESA_LOADER_DRIVER_OVERRIDE=llvmpipe WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1
  log ">> [AppImage] 6. sin GLib empaquetada (usa la del sistema)"
  ( ./app.AppImage --appimage-extract >/dev/null 2>&1 )
  if [ -d squashfs-root ]; then
    find squashfs-root -type f \( -name 'libglib-2.0.so*' -o -name 'libgio-2.0.so*' -o -name 'libgobject-2.0.so*' -o -name 'libgmodule-2.0.so*' \) -delete
    run "[AppImage] 6. sin GLib empaquetada" squashfs-root/AppRun WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1
  fi
fi

log ">> [.deb] extraigo (sin instalar) y corro su binario (pista rápida; la prueba REAL del .deb es instalarlo)"
if dpkg-deb -x app.deb debroot 2>/dev/null; then
  DEBBIN=$(find debroot -type f -name 'brisvia-miner' | head -1)
  [ -n "$DEBBIN" ] && run "[.deb] binario extraído" "$DEBBIN" WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 || log "   (no encontré el binario)"
fi
log "   NOTA .deb: para la validación FINAL del .deb, instalalo de verdad en esta máquina (o una descartable):"
log "        sudo apt install ./app.deb   (resuelve dependencias del sistema)  y luego abrí:  brisvia-miner"

# guardar la evidencia local
cp "$OUT" "$DEST" 2>/dev/null
log ""
log "=================================================================="
log " Evidencia guardada en: $DEST"
log "=================================================================="

echo ""
echo "IMPORTANTE: 'SIN ERROR FATAL' no garantiza que la ventana se haya visto (podría estar negra)."
echo "Durante las pruebas, ¿alguna abrió una ventana de Brisvia REAL y visible (no negra)?"
read -r -p "  Escribí el número de variante que se vio bien (o 'ninguna'): " VIS
echo "Respuesta del usuario (variante visible): $VIS" >> "$OUT"; cp "$OUT" "$DEST" 2>/dev/null

echo ""
read -r -p "¿Querés SUBIR la evidencia a un enlace para pasársela fácil a Claude? (s/n): " SUB
if [ "$SUB" = "s" ] || [ "$SUB" = "S" ]; then
  URL=$(curl -fsSL -F "file=@$DEST" https://0x0.st 2>/dev/null)
  [ -z "$URL" ] && URL=$(curl -fsSL --upload-file "$DEST" https://transfer.sh/brisvia-diag.txt 2>/dev/null)
  if [ -n "$URL" ]; then echo ">> Pasale ESTE ENLACE a Claude:  $URL"; else echo ">> No se pudo subir. Mandale el archivo: $DEST"; fi
else
  echo ">> Ok. Mandale a Claude el archivo:  $DEST  (o abrilo, copiá todo y pegalo)."
fi
cd /; rm -rf "$WORK"
