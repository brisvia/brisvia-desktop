# Matriz de continuidad 1.1.0/1.1.1/1.1.2 → 1.1.3 (R = 4739d298)

Auditoría sistemática pedida por Fernando ("no puede ser que sigamos encontrando cosas") sobre las 26 categorías propuestas por ChatGPT. Estado verificado con git + VPS el 28-jul-2026.

**Identidad congelada:** R = `4739d298c43c6f759d36ae44f86663487b2c417a` (HEAD `release/1.1.3`, version 1.1.3 en tauri.conf + Cargo.toml + package.json). Padre `58286e2f`; diferencia = 3 comentarios (cero funcional). v1.1.1 (`5df3ec1`) es ancestro de R. La 1.1.2 de producto real = `2bb8a78` (no el tag `eeddf56`, mal puesto en rama lateral de CI).

| # | Categoría | Estado | Evidencia / acción |
|---|-----------|--------|--------------------|
| 1 | Canales de actualización (GitHub + mirror China) | CERRADO | endpoints GitHub idéntico en v1.1.1/2bb8a78/R; MIRROR_ENDPOINT China en lib.rs L4262 + fallback L4317. Pubkey idéntica todas las versiones (md5 d191f7be). |
| 2 | Identidad de versión | CERRADO | R = 1.1.3 en los 3 archivos. Tag `v1.1.3` apunta a commit viejo `ffc31af` → **se mueve a C'=R en la ceremonia del OK** (documentado). |
| 3 | Firmas y confianza | CERRADO | pubkey minisign continua → apps viejas validan 1.1.3. Assets R firmados (Authenticode Valid, updater sig VALID en gates). |
| 4 | Datos / billetera | CERRADO | fix walletdir + migrate_legacy + prepare_wallet_layout en R (heredado de la 1.1.2 real 2bb8a78). Gate 4 (1.1.1→1.1.3 y 1.1.2→1.1.3 conservan wallet) verde. |
| 5 | Fair launch y tiempo | CERRADO | MAINNET_START 1785596400 + barrier + extended_readiness + wait-mode (app.js). 4 pins fair-launch en el camino de R. P0#1 validado 98%. |
| 6 | Identidad core y sidecars | CERRADO | bitcoind/cli/worker pin core@8195a1b; genesis aa6bc268 verificado por plataforma. |
| 7 | Minería SOLO | CERRADO | Gate 7 testnet aislado: getblocktemplate→RandomX→submitblock→bloque aceptado (FAST y LIGHT). |
| 8 | Minería POOL | CERRADO | OFFICIAL_POOL_URL pool.brisvia.com:3333 + BRISVIA_POOL_URL custom; supervisor pool_url fijo (nunca POOL→SOLO). Pool E2E release-critical cerrado. |
| 9 | Seeds / descubrimiento | CERRADO | 3 nodos binario 38564f06 (candados, genesis aa6bc268), cross-connect OK, firewall 9342 abierto / RPC 9338 cerrado (28-jul). |
| 10 | Explorer | CERRADO | transición /root/brisvia-explorer-mainnet.sh L64-71 crea mining-pools-configs/BRVA (mkdir + 4 JSON) al arrancar en T0. explorer-mainnet.timer → 01-ago 15:00. |
| 11 | Web y descargas | PREPARADO | web 5 idiomas → 1.1.3 en copia /tmp/web-113 (hashes R). snippet /download/* preparado a v1.1.3. Deploy con el OK. |
| 12 | i18n / contenido | CERRADO | locales.js en R (5 idiomas). Cartel updater inglés por ítems (aprobado). Changelog in-app 5 idiomas → 1ª actualización post-lanzamiento (decidido). |
| 13 | Instaladores / lifecycle | CERRADO | Gate test-installer-scenarios verde (unicode-datadir, brisvia-plus-foreign). |
| 14 | Linux / AppImage | CERRADO | fix AppImage full matrix (22.04/Debian12/24.04/26.04/Fedora44) en R vía 2bb8a78/1acd21d + Debian 13. Está en el asset R real. |
| 15 | Windows | CERRADO | setup.exe R Authenticode Valid, SHA eac3c27d = manifiesto. Sidecars firmados (a80388d). |
| 16 | macOS | CERRADO | arm64 verificado. app.tar.gz updater + dmg descarga. Smoke CI. |
| 17 | Seguridad Tauri | CERRADO | CSP script-src 'self', RPC nodo 127.0.0.1, Command::new controlados. Sin agujeros. |
| 18 | RPC y permisos | CERRADO | pool usa rpcuser-pagos + rpcuser2 peer separados, rpcport 9338; explorer cookie. |
| 19 | Pool backend / ceremonia T0 | CERRADO | artefacto congelado /opt/brisvia-pool/current (MANIFIESTO + owner_auth), pool-guard/launch/open, fail-closed, timers launch/expire/watchdog. |
| 20 | Systemd / arranque | CERRADO | 0 units failed; timers T0 correctos; sobrevive reboot (3 reboots + firewall persistence service enabled). |
| 21 | Firewall / red | CERRADO | firewall 9342 abierto (P2P), RPC 9338 privado; pool 3333 persistence service enabled (fail-closed, sobrevive reboot). |
| 22 | DNS / TLS / dominios | CERRADO | brisvia.com/updates/latest.json TLS externo 200; pool/explorer.brisvia.com vivos. Mirror sirve desde /var/www (fijo, sobrevive deploys). |
| 23 | Monitoreo / alertas | CERRADO | cron 5min 3 nodos + alerta Discord HTTP 200 probada; freeze-watch baseline = 38564f06. |
| 24 | Backups / recuperación | CERRADO | ledger snapshot/backup timers activos; A2 restore otra instalación recupera 50 BRVA (validado). |
| 25 | CI/CD y release | CERRADO | 8/8 gates sobre R; 3 CI verdes; manifiesto release-go c4908031. Higiene de tag documentada. |
| 26 | Comunicación / soporte | PREPARADO | bodies release 1.1.3 + 1.1.2 (inglés, sin exponer errores) en borrador; noticia Discord 5 idiomas lista. Se aplican con el OK. |

## Mirror China 1.1.3 — staging (el punto que se había escapado, ya cerrado)
- Binarios de R en `/var/www/updates/v1.1.3/` — **5 formatos** (setup.exe, app.tar.gz, AppImage, dmg, deb), SHA verificados = R + los .sig.
- `latest.json.NEW-1.1.3` preparado (firmas de R = las de GitHub, urls al VPS). **Activo sigue en 1.1.1** — se activa con `mv` en el OK.
- **Verificado desde red externa** (brisvia.com, DNS público): 200, `application/octet-stream` (no HTML, no redirect a GitHub), SHA = R, firma minisign VÁLIDA contra la pubkey del updater (keyid 5a9fe07b58bf4a00) + firma global válida.
- La descarga directa manual para chinos en 1.0.9/1.1.0 (sin fallback en el updater) **existe**: `brisvia.com/updates/v1.1.3/<archivo>`. Falta solo documentarla en la web (paso del deploy).

## Gaps ChatGPT (cierre continuidad 28-jul)
- **G2 descarga externa**: CERRADO (SHA/firma/MIME/no-HTML verificados desde otra red).
- **G1 firma del mirror**: CERRADO criptográficamente (minisign válida). La canaria de la APP real (1.1.1 con GitHub bloqueado → fallback → instala R) necesita Windows + app: la puede correr Fernando; el code-path de fallback está en lib.rs y el gate verify-update ya pasó.
- **G3 descarga manual China**: binarios completos en el mirror; documentar en /downloads o /blocked durante el deploy web (5 idiomas).
- **G4 tag 1.1.2**: NO se mueve. Solo se documenta la procedencia (commit 2bb8a78) en su body. Bodies 1.1.3 + 1.1.2 preparados con "Build provenance".

## Secuencia del OK único (afinada por ChatGPT — nada se dispara sin el OK verbatim de Fernando)
1. Fijar `notes` y `pub_date` en los DOS latest.json (GitHub + mirror); misma versión/firmas/plataformas, solo difieren las URLs.
2. Congelar hashes de manifiestos, web, mensajes y launch-bundle → Fernando aprueba el SHA exacto del bundle.
3. Crear C' (= R + 2 JSON), promover main, **mover tag anotado v1.1.3 → C'** con lease. Corregir el draft. (El tag v1.1.2 NO se toca.)
4. Verificar desde clon limpio + gate seco.
5. `publish-approved-release` (release público con assets R). Verificar tag/source archives/11 assets/hashes/firmas públicamente.
6. Canaria pública del canal GitHub.
7. **Activación atómica** de los 2 latest.json (GitHub + `mv latest.json.NEW-1.1.3 latest.json`); verificar desde redes distintas; una actualización real posterior NO vuelve a ofrecer 1.1.3.
8. Activar snippet /download/* a v1.1.3, deploy web 5 idiomas (con la nota de descarga directa mirror) + purgar cachés.
9. Aplicar bodies 1.1.3 + 1.1.2 (procedencia honesta).
10. **Reset SOLO contadores de descarga de GitHub**, con backup previo. NUNCA ledger/pagos/wallet/estadísticas históricas.
11. Discord solo cuando updater + descargas estén comprobados.

## Pruebas finales pre-OK (28-jul, todas verde)
- **Seeds T0 (3)**: ExecStart = bitcoind 38564f06 (candados) idéntico en .145/.36/.102; ExecStartPre check-node-sha.sh valida ese SHA, dry-run exit 0 (no frena T0); timer 01-ago 15:00 UTC; mainnet+pool inactive; NTP sync; datadir mainnet separado. **Era el único bloqueante técnico de ChatGPT → cerrado en vivo.**
- **latest.json x2**: GitHub + mirror con misma versión/notes/firmas; GitHub→github, mirror→VPS; JSON válido sin duplicadas.
- **Web**: 137 archivos, 5 idiomas, home/downloads/blocked, 1.1.2 residual=0, hashes R en 5 idiomas, 0 links al draft, aliases /download/* + snippet v1.1.3, `nginx -t` OK. web_tree_sha 90bdf814.
- **Git dry-run**: release/1.1.3=R, main=155caa5, tag→f16ff5e (a mover a C'), manifiestos presentes, R es HEAD. 6 checks de la ceremonia claros.
- **Congelado en launch-bundle**: frozen_hashes (web/snippet/mirror/bodies/latest.json), reset_counters (solo descargas GitHub), rollback (git/latest.json x2/web/snippet). Validador: 3 avisos = solo pub_date+authorization (momento-del-OK).

## Residuos identificados (NO tocar)
`/var/www/brisvia/` (web vieja 1.0.9, no servida), `/var/www/brisvia-releases/*` (backups web), `.bak` de units, tag v1.1.2 mal puesto (no re-referenciar, no mover).
