# Runbook de lanzamiento — Brisvia 1.1.3 · T0 = 2026-08-01 15:00:00 UTC (12:00 ART)

Guía operativa del lanzamiento. Un **único OK de Fernando** autoriza toda la secuencia; **cada freno (STOP) puede detenerla**. Nada se ejecuta mientras el paquete de lanzamiento (`launch-bundle-v1.1.3.json`) no esté en estado `READY_FOR_APPROVAL` y congelado.

Referencias: `launch-bundle-v1.1.3.json` (hashes + evidencia), `release-go-v1.1.3.json` (build R), `owner-approval-v1.1.3.json` (solo publica el release).

---

## 0. Requisitos ANTES de pedir el OK (checklist para pasar a READY)

- [x] Build R (`4739d298`) — 8/8 gates verdes; sidecars con candados; assets con SHA (en el bundle).
- [x] Fair-launch: R rechaza bloque-1 antes de T0 / lo acepta después (demostrado). La app 1.1.1 tiene wait-mode.
- [x] Pool E2E release-critical: share válida/negativas/reconexión/suspensión/compat; nunca cae GRUPO→SOLO.
- [x] Actualización automática: 1.0.9 / 1.1.0 / 1.1.1 / 1.1.2 → R (todas verdes).
- [x] Arquitectura macOS = arm64 (clave `darwin-aarch64` correcta).
- [x] Explorador: fail-closed + rollback + health check por `/api/block/0` + dir BRVA persistente + timer reintenta ~2h.
- [x] Pool pública NO entrega trabajo mainnet antes de T0 (triple candado).
- [ ] **latest.json final**: completar `notes` (cartel aprobado por Fernando) + `pub_date` UTC → recalcular SHA → validar con la app 1.1.1 real → verificar que cada URL devuelve los bytes del SHA esperado (no HTML/redirect) → MIME/caché/anónimo.
- [ ] **Reboots** de los 3 servidores + manifiestos post-reboot (sección 3).
- [ ] **Monitoreo/alertas probados de verdad** (disparar una alerta sintética y confirmar que llega al canal).
- [ ] **Web** congelada (commit + hash) y **Discord** congelado (mensaje + hash).
- [ ] Reordenamiento Git **ensayado en seco** (ya: 6 checks OK) — sin ejecutar.

Cuando todo esto esté: `launch-bundle` pasa a `READY_FOR_APPROVAL`, se **congela** y se calcula su SHA-256.

---

## 1. El OK único y la secuencia (se ejecuta con gates entre etapas)

1. **Fernando da el OK** referenciando el SHA del `launch-bundle` congelado → se crea `launch-approval-v1.1.3.json` (referencia ese SHA) + se completa `owner-approval-v1.1.3.json` (verbatim).
2. **Reordenamiento Git**: crear C' (= R + 2 manifiestos) → merge M (árbol = C') → promover `main` → mover tag `v1.1.3` a C' (`--force-with-lease` contra el tag viejo `ffc31af0`) → corregir `target_commitish` del draft.
3. **Clon limpio + gate en seco** (verificar identidad/reproducibilidad sin publicar).
4. **Publicar el GitHub Release v1.1.3** (`publish-approved-release`, `make_latest=true` del release; el `latest.json` del updater se activa aparte).  ⟶ **STOP** si falla.
5. **Verificación anónima**: descargar los assets desde una IP/entorno sin credenciales, verificar SHA.
6. **Canaria del canal público**.  ⟶ **STOP** si falla (no activar el updater).
7. **Activar `latest.json`** (publicación atómica; nunca sobrescribir el archivo servido en caliente). Guardar copia hasheada del `latest.json` de 1.1.1 por si hay que frenar.
8. **Update real post-publicación**: probar 1.1.1→R y 1.1.2→R contra el manifiesto activo.  ⟶ **STOP + revertir latest.json a 1.1.1** si falla.
9. **Web** (commit congelado) y **Discord** (mensaje congelado) — recién ahora, con el release ya vivo y verificado.
10. **Red mainnet cerrada** hasta el momento autorizado. En T0, los timers arrancan los servicios y se ejecuta la sección 4.

---

## 2. Frenos de seguridad (STOP gates)

- Falla la **publicación del release** → detener, no activar `latest.json`.
- Falla la **canaria pública** → detener, no activar `latest.json`.
- Falla el **update real 1.1.1→R** → detener, revertir `latest.json` a 1.1.1.
- Aparece un **listener inesperado en 9342/9338** antes de T0 → alerta + investigar antes de seguir.
- El **explorador no sirve el génesis** `aa6bc268` en la transición → rollback automático (ya en el script).
- Cualquier duda material → NO avanzar a web/Discord.

---

## 3. Reboots controlados de los 3 servidores (ANTES de T0)

Orden: **Oracle .36 → Oracle .102 → Hostinger .145** (el último, coordinado: interrumpe testnet/explorador/pool testnet). Esperar un rato estable entre cada uno.

**Antes de cada reboot** guardar evidencia (manifiesto del nodo):
```
date -u; uptime; systemctl --failed
systemctl list-timers --all | grep -Ei 'brisvia|pool|explorer'
pgrep -a bitcoind; ss -lntup; timedatectl; free -h; swapon --show; df -h
sha256sum <RUTA_CANONICA>/bitcoind
systemctl cat brisvia-mainnet.service | sha256sum
systemctl cat brisvia-mainnet.timer | sha256sum
```
Confirmar antes de reiniciar: mainnet service inactivo · timer activo con T0 correcto · sin listener 9342/9338 · sin updates de paquetes pendientes.

**Después de cada Oracle** (resultado obligatorio pre-T0): 0 bitcoind mainnet · mainnet inactive · **un solo** timer activo con próxima ejecución **exactamente en T0** · sin listener 9342/9338 · NTP sincronizado · swap presente · ninguna unit duplicada/fallida · `check-node-sha.sh` aprueba · firewall/Security List sigue permitiendo P2P y cerrando RPC.

**Antes de .145** guardar además: estado/altura/génesis testnet, estado explorer, estado pool testnet, config nginx, DB/cache explorer, units de transición.
**Después de .145**: testnet vuelve solo (RPC 19332, P2P 19333) · explorer vuelve sobre testnet · mainnet inactive · 9342/9338 sin listener · timer mainnet en T0 · el script del explorer conserva el fix · sin procesos `(deleted)` (`lsof +L1 | grep -Ei 'bitcoind|brisvia|explorer|stratum'`) · sin symlinks rotos (`find /etc/systemd/system -xtype l`). **Si .145 no recupera solo → el bundle NO pasa a READY hasta corregirlo.**

**Firewall del 3333 — persistencia RESUELTA (automatica)**: el service `brisvia-pool-firewall.service` (enabled, oneshot, corre al boot despues de ufw) re-pone la regla `iptables ... --dport 3333 ! -s 127.0.0.1 -j DROP` en cada arranque, SOLO si la pool no esta abierta (POOL_PUBLIC ausente) — fail-closed. Ya NO hay que re-aplicarla a mano. Verificado el 28-jul con un reinicio real: la regla queda puesta sola y el 3333 publico queda cerrado. El guard vive en `/usr/local/bin/brisvia-pool-firewall-guard.sh` (FUERA del artefacto congelado de la pool). Nota: la pool testnet era un ensayo manual (0 mineros) y quedó retirada; `pool.brisvia.com` NO responde hasta T0 (esperado).

---

## 4. En T0 (arranque real)

- Los timers arrancan `brisvia-mainnet.service` (nodo seed) en los 3.
- A T0+1min el `brisvia-explorer-mainnet.timer` dispara la transición (reintenta cada minuto ~2h).
- La pool mainnet abre por su gate automático (ventana T0..T0+45), tras canaria.
- Verificar: los 3 seeds en la cadena correcta (génesis aa6bc268), P2P 9342 abierto, RPC 9338 cerrado desde fuera.

## 5. Post-T0 (solo lo que exige la red real)

Propagación del primer bloque · explorer mostrando el bloque 1 · pool encontrando un bloque real · madurez de coinbase · primer payout real · monitoreo sostenido 6/24 h.

---

## Riesgos residuales aceptados
- **Pre-cómputo privado** de RandomX: inherente a cualquier PoW, no prevenible por el candado (que sí evita la aceptación temprana por nodos honestos). No mitigado por el 0% premine (propiedad distinta). Comunicar: *"los nodos 1.1.3 no aceptan bloques antes de T0"*, no "es imposible minar antes".
- **Historial git antiguo** con menciones de IA: el árbol actual / candidato publicado están limpios; la historia vieja no se reescribe. Afirmación pública: "la versión publicada está limpia", no "todo el historial".
- **Verificaciones que solo ocurren después de T0** (ver sección 5).
