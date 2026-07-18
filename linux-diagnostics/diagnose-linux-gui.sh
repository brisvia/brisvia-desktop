#!/usr/bin/env bash
# Brisvia Miner - Linux GUI diagnostic (Ubuntu 24.04 / 26.04, Intel/Wayland).
#
# Why this exists: on some Intel + Wayland setups the window never opens. Two independent causes were seen on
# a ThinkPad T430 / Intel HD 4000 / Ubuntu 26.04:
#   (1) GLib/GVFS clash  -> "undefined symbol: g_task_set_static_name" (the .AppImage bundles an OLD GLib that
#       the host's NEWER GVFS modules can't use). The .deb does NOT bundle GLib, so it avoids this entirely.
#   (2) EGL init failure -> "Could not create default EGL display: EGL_BAD_PARAMETER. Aborting..." (WebKitGTK
#       can't start EGL on this GPU under Wayland). Disabling the DMABUF renderer + compositor works around it.
#
# What this does: runs the SAME .AppImage under several environment combinations, a few seconds each, and
# reports which one actually opened a window (no fatal EGL/GLib error). It changes NOTHING on your system,
# never touches your wallet/data, and kills each attempt automatically. Send back the file it writes.
#
# Usage:  ./diagnose-linux-gui.sh /path/to/Brisvia-Miner-Linux.AppImage
# Output: ./brisvia-linux-diagnostic-<host>.txt   (copy its full content back to us)

set -u
AI="${1:-}"
if [ -z "$AI" ] || [ ! -f "$AI" ]; then
  echo "Uso: $0 /ruta/al/Brisvia-Miner-Linux.AppImage"
  exit 2
fi
chmod +x "$AI" 2>/dev/null || true
OUT="./brisvia-linux-diagnostic-$(hostname 2>/dev/null || echo host).txt"
SECS=8   # cuántos segundos dejamos vivo cada intento antes de matarlo
: > "$OUT"

log(){ echo "$@" | tee -a "$OUT"; }

log "==================================================================="
log " Brisvia Miner - diagnostico de interfaz en Linux"
log " Fecha (UTC): $(date -u 2>/dev/null)"
log " AppImage   : $AI"
log " SHA-256    : $(sha256sum "$AI" 2>/dev/null | cut -d' ' -f1)"
log "==================================================================="
log ""
log "--- Entorno ---"
log "Sesion        : ${XDG_SESSION_TYPE:-desconocida} (Wayland/x11)"
log "Distro        : $( (. /etc/os-release 2>/dev/null; echo "$PRETTY_NAME") )"
log "Kernel        : $(uname -r 2>/dev/null)"
log "GLib (host)   : $(pkg-config --modversion glib-2.0 2>/dev/null || ldconfig -p 2>/dev/null | grep -m1 libglib-2.0 | sed 's/.*=> //')"
log "GPU (glxinfo) : $(command -v glxinfo >/dev/null 2>&1 && glxinfo 2>/dev/null | grep -m1 'OpenGL renderer' || echo 'glxinfo no instalado')"
log ""

# Detecta en un log si hubo falla fatal conocida o si abrio bien.
verdict(){
  local logf="$1"
  if grep -qi "EGL_BAD_PARAMETER" "$logf"; then echo "FALLA: EGL_BAD_PARAMETER"; return; fi
  if grep -qi "g_task_set_static_name" "$logf"; then echo "AVISO: conflicto GLib/GVFS"; fi
  if grep -qiE "Could not create default EGL|Aborting|cannot open display|Segmentation fault" "$logf"; then echo "FALLA: no inicio los graficos"; return; fi
  echo "OK: parece haber abierto la ventana (sin error fatal en $SECS s)"
}

run_variant(){
  local name="$1"; shift
  local logf; logf="$(mktemp)"
  log "==================================================================="
  log ">> VARIANTE: $name"
  log "   env: $*"
  # Lanzamos con las env pasadas como 'CLAVE=valor' antes del binario.
  ( env "$@" "$AI" ) >"$logf" 2>&1 &
  local pid=$!
  sleep "$SECS"
  # matar el arbol de procesos del intento
  kill "$pid" 2>/dev/null; sleep 1; kill -9 "$pid" 2>/dev/null
  pkill -f "$(basename "$AI")" 2>/dev/null
  log "   RESULTADO: $(verdict "$logf")"
  log "   --- ultimas 12 lineas del intento ---"
  tail -12 "$logf" | sed 's/^/   | /' | tee -a "$OUT" >/dev/null
  rm -f "$logf"
  log ""
}

# 1) Tal cual (reproduce el problema)
run_variant "1. AppImage sin cambios"
# 2) Solo DMABUF off
run_variant "2. WEBKIT_DISABLE_DMABUF_RENDERER=1"         WEBKIT_DISABLE_DMABUF_RENDERER=1
# 3) DMABUF + compositor off  (este es el DEFAULT que ya trae el arreglo del programa)
run_variant "3. DMABUF + COMPOSITING off (arreglo nuevo)" WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1
# 4) Forzar X11 (XWayland)
run_variant "4. + GDK_BACKEND=x11"                        GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1
# 5) Render por software (ultimo recurso, lento pero abre)
run_variant "5. + LIBGL_ALWAYS_SOFTWARE=1"                LIBGL_ALWAYS_SOFTWARE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1

# 6) AppImage con la GLib empaquetada QUITADA (usa la del sistema) -> prueba la causa 1
log "==================================================================="
log ">> VARIANTE 6: AppImage con GLib del sistema (se quita la GLib empaquetada)"
WORK="$(mktemp -d)"
( cd "$WORK" && "$OLDPWD/$AI" --appimage-extract >/dev/null 2>&1 || "$AI" --appimage-extract >/dev/null 2>&1 )
if [ -d "$WORK/squashfs-root" ]; then
  # Quitar SOLO la familia GLib (esta en la excludelist oficial de AppImage): que se tome del host.
  find "$WORK/squashfs-root" -type f \( -name 'libglib-2.0.so*' -o -name 'libgio-2.0.so*' \
       -o -name 'libgobject-2.0.so*' -o -name 'libgmodule-2.0.so*' \) -print -delete \
       | sed 's/^/   quitado: /' | tee -a "$OUT" >/dev/null
  logf6="$(mktemp)"
  ( env WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 "$WORK/squashfs-root/AppRun" ) >"$logf6" 2>&1 &
  pid6=$!; sleep "$SECS"; kill "$pid6" 2>/dev/null; sleep 1; kill -9 "$pid6" 2>/dev/null; pkill -f AppRun 2>/dev/null
  log "   RESULTADO: $(verdict "$logf6")"
  log "   --- ultimas 12 lineas ---"
  tail -12 "$logf6" | sed 's/^/   | /' | tee -a "$OUT" >/dev/null
  rm -f "$logf6"
else
  log "   (no se pudo extraer el AppImage; salteo esta variante)"
fi
rm -rf "$WORK"
log ""
log "==================================================================="
log " FIN. Envianos el archivo:  $OUT"
log "==================================================================="
echo "Listo. Mandanos el contenido de: $OUT"
