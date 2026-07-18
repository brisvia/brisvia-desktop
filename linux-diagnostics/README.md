# Brisvia Miner — la ventana no abre en Linux (Intel + Wayland)

Diagnóstico y arreglo del caso reportado el 2026-07-18: en un **ThinkPad T430 / Intel HD 4000 / Ubuntu 26.04
(Wayland)**, el `.AppImage` 1.0.8 no abre la interfaz. Son **dos fallas independientes**:

| # | Síntoma en el log | Causa | Afecta a |
|---|---|---|---|
| 1 | `undefined symbol: g_task_set_static_name` → `Failed to load module libgvfsdbus.so` | El `.AppImage` empaqueta una **GLib vieja** (2.72). En Ubuntu 24.04/26.04 los módulos GVFS del sistema son nuevos (GLib 2.76+) y no encuentran el símbolo → chocan. | Solo el `.AppImage` (el `.deb` no empaqueta GLib). |
| 2 | `Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...` | WebKitGTK no logra iniciar EGL en esa GPU bajo Wayland. | `.AppImage` **y** `.deb`. |

## Los tres arreglos

### A. EGL — APLICADO en el código (cubre AppImage y .deb)
En `src-tauri/src/lib.rs`, al inicio de `run()` (antes de crear la ventana), en Linux se fijan por defecto:

    WEBKIT_DISABLE_DMABUF_RENDERER=1
    WEBKIT_DISABLE_COMPOSITING_MODE=1

Solo si el usuario no las definió (así un avanzado puede sobrescribir). **No** se fuerza `GDK_BACKEND=x11`
para no romper equipos donde Wayland ya anda. Esto hace que WebKitGTK caiga a un camino que sí inicia EGL en
Intel/Wayland. Cubre las dos formas de instalar (AppImage y .deb) sin que el usuario haga nada.

> Falta la prueba final en hardware real (Intel HD 4000 + Wayland). El default es seguro y es la combinación
> recomendada para WebKitGTK; que ABRA la ventana en ese equipo hay que confirmarlo con el diagnóstico de abajo.

### B. GLib (causa 1) — recomendación: usar el `.deb` en Ubuntu nuevo
El `.deb` **no empaqueta GLib**: usa la del sistema, así que el choque GLib/GVFS **no existe**. Es el camino
más robusto para Ubuntu 24.04/26.04. El `.deb` ya se genera en cada build (`targets: ["appimage","deb"]`).
Acción pendiente (sin publicar): recomendar el `.deb` como instalación preferida para Ubuntu en la web/descargas.

### C. GLib (causa 1) — candidato para el `.AppImage`: `repack-appimage-without-glib.sh`
Quita del AppImage solo la familia GLib (que está en la excludelist oficial de AppImage) para que se tome del
host. Razonado, no a ciegas: la app se compila en 22.04 (GLib 2.72) y todo host destino tiene GLib ≥ 2.72
(compatible hacia adelante). **CANDIDATO**: no está cableado a la receta que publica releases; hay que probarlo
en 22.04/24.04/26.04 antes de shipearlo.

## Diagnóstico en el equipo afectado (una sola corrida)
El dueño del equipo corre:

    chmod +x diagnose-linux-gui.sh
    ./diagnose-linux-gui.sh /ruta/al/Brisvia-Miner-Linux.AppImage

Prueba el AppImage bajo 6 combinaciones (tal cual / DMABUF off / DMABUF+compositor off / +X11 / +software /
AppImage sin GLib empaquetada), unos segundos cada una, sin tocar billetera ni datos, y escribe
`brisvia-linux-diagnostic-<host>.txt`. Con ese archivo sabemos EXACTO qué combinación abre la ventana y cuál de
los arreglos hay que dejar como definitivo.

## Matriz de validación (pendiente — necesita hardware/entorno real)
No reproducible desde la máquina de build (Windows) ni en CI headless (la falla EGL depende de GPU + Wayland
reales). Falta correr:

| Distro | AppImage | AppImage sin GLib | .deb |
|---|---|---|---|
| 22.04 | ¿abre? | no romper | ¿abre? |
| 24.04 | (causa 1) | ¿abre? | ¿abre? |
| 26.04 | (causa 1+2) | ¿abre? | ¿abre? |

Restricciones respetadas: no se publica release, no se reemplaza la 1.0.8, no se toca `latest.json`. Todo en la
rama `fix/ubuntu-2604-appimage`. Nada se da por corregido hasta tener prueba real en el equipo Intel/Wayland.
