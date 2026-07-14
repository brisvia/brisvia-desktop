# PRODUCT_CONTRACTS.md — qué es verdad en cada pantalla

Lo que un test unitario no ve: **si el texto que aparece tiene sentido para quien lo está viendo**.

Fernando encontró un cartel que ofrecía "crear tu billetera" en una pantalla que **sólo ve quien ya la tiene**.
Ningún barrido mecánico lo detecta: las claves estaban completas, sin mezcla de idiomas, todo verde. Hace falta
saber **quién ve esa pantalla y cuándo**. Eso es lo que este archivo escribe.

Cada contrato dice qué **debe** verse y qué está **prohibido** ver. Lo prohibido es la parte que importa: es lo
que nadie revisa.

---

## Estados y contratos

### Sin billetera (primer arranque)
- **Se ve:** el onboarding (bienvenida → crear/importar → contraseña → 12 palabras → verificación).
- **PROHIBIDO:** la pantalla de espera, la de desbloqueo, cualquier saldo, la billetera principal.
- **Nota:** el backend decide por el archivo en disco (`wallet_seed_on_disk`), no por si el nodo responde.
  Si falla la consulta, asumir QUE SÍ HAY billetera (fail-closed): mostrar el onboarding a quien ya la tiene
  invita a sobrescribirla.

### Con billetera, antes del 1-ago-2026 15:00 UTC (modo espera)
- **Se ve:** la pantalla de espera, la cuenta regresiva, la billetera usable.
- **PROHIBIDO:** cualquier texto que invite a CREAR una billetera (ya la tiene). ← *el bug de Fernando*
- **PROHIBIDO:** el estado "Sincronizando" (confunde: todavía no hay red). Va "En espera de lanzamiento".
- **PROHIBIDO:** que el botón de minar esté activo.

### Con billetera, después del lanzamiento
- **Se ve:** la billetera, el minado habilitado, el estado de red real.
- **PROHIBIDO:** "Conectada" con cero pares. ← estado imposible
- **PROHIBIDO:** decir que mina si el worker no está corriendo.

### Contraseña
- **Regla única:** mínimo **6** caracteres. El cartel y el backend dicen EXACTAMENTE lo mismo.
  ← *el bug de Fernando: la pantalla decía 8, el backend exigía 12*
- **PROHIBIDO:** que la pantalla anuncie un mínimo distinto al que el backend aplica.
- La barra de fuerza es **sugerencia**, nunca obligación.

### Modo de minado (1.0)
- **Se ve:** "En solo" activo. Pool y otra pool **deshabilitados**, con el motivo escrito.
- **PROHIBIDO:** que los botones de pool reaccionen (`POOL_ENABLED = false` manda; la pantalla sólo lo refleja).
- **PROHIBIDO:** que la pantalla ofrezca pool si el backend no lo permite. La config es un archivo editable:
  la pantalla sola no alcanza como candado.

### El nodo no arranca
- **Se ve:** un mensaje que dice qué pasó y qué hacer, en el idioma del usuario.
- **PROHIBIDO:** texto crudo del nodo en inglés. ← *le apareció "node is not ready yet"*
- **PROHIBIDO:** quedarse para siempre en "Preparando billetera" (eso es lo que pasaba con la base dañada).
- Disco lleno / datadir bloqueado / permisos: **se informan, NO se "reparan"** (reparar no los arregla y hace
  un bucle).

---

## Términos oficiales (una sola palabra por concepto)

| Concepto | ES | EN | NO usar |
|---|---|---|---|
| Grupo de minado | **pool** | **pool** | ~~grupo~~, ~~group~~ ← *el bug de Fernando* |
| Moneda | Brisvia / BRVA | Brisvia / BRVA | — |
| Palabras de recuperación | 12 palabras | 12 words | ~~seed~~, ~~semilla~~ (al usuario) |
| Red real | red real / mainnet | real network | — |

---

## Fuentes de verdad (de dónde sale cada dato)

| Dato | Fuente ÚNICA | Nunca |
|---|---|---|
| **Versión** | `app_version()` → `CARGO_PKG_VERSION` → `runningVersion` | escrita a mano en HTML/JS, ni leída del DOM ← *bug real: se leía del cartel de pantalla* |
| **Red** | el backend (`netcfg`) | inferida en el frontend |
| **Modo de minado** | el backend (`POOL_ENABLED` + `mining_mode`) | lo que diga localStorage |
| **Hay billetera** | el archivo en disco | que el nodo responda |
| **Pares / altura / dificultad** | el nodo por RPC | cacheada en el frontend |
| **Share aceptada** | la confirmación explícita de la pool | haberla enviado |

**Regla general:** el DOM nunca es fuente de estado. La pantalla muestra lo que el backend dice; no lo decide.

---

## Reglas de formato

| | ES | EN |
|---|---|---|
| Decimales | coma (`0,00`) | punto (`0.00`) |
| Fecha del lanzamiento | 1 de agosto de 2026, 15:00 UTC (12:00 en Argentina) | August 1, 2026 at 15:00 UTC |
| Unidades (`H/s`) | igual en los dos | igual en los dos |

Los botones de idioma van **cada uno en su propio idioma** ("Español" / "English"): eso es correcto, no una
mezcla.
