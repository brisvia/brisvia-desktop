# INCIDENT_PATTERNS.md — cada error, una defensa permanente

Una entrada por incidente. Sin narraciones. La pregunta que importa es **"por qué los controles que ya existían
no lo vieron"**, porque esa respuesta es la defensa nueva.

Regla: un incidente sin defensa automática asociada **no está cerrado**. "Acordarse de revisar X" no es una
defensa.

---

### 1. La pantalla decía 8 caracteres, el backend exigía 12
- **Invariante violada:** el cartel y el backend tienen que aplicar la MISMA regla.
- **Por qué no se detectó:** cada mitad era correcta por separado. El bug vive en la costura. Nadie ejecutó
  "crear una billetera" de punta a punta.
- **Agravante:** diagnostiqué sin reproducir ("es un cartel residual, apretá Continue"). Fernando perdió tiempo
  confirmando mi hipótesis equivocada.
- **Defensa:** `product_first_contact` escenario A paso 3 (probar con un carácter MENOS que el mínimo).
- **Riesgo que queda:** la rutina es manual hasta que el E2E la cubra.

### 2. Texto en español con el idioma en inglés
- **Invariante violada:** todo texto visible sale del sistema de idiomas.
- **Por qué no se detectó:** lo escribí a mano en el HTML ese mismo día. El i18n sólo ve lo marcado con
  `data-i18n`: lo escrito a mano es invisible para él.
- **Defensa:** `tools/check_textos.py` en el CI (falla si hay texto fijo sin traducir; lista blanca corta y
  explícita).
- **Control negativo:** probado reintroduciendo el placeholder en español → falla.

### 3. "Podés crear tu billetera" en una pantalla que sólo ve quien ya la tiene
- **Invariante violada:** el texto tiene que ser verdad PARA ESE ESTADO.
- **Por qué no se detectó:** ningún barrido mecánico lo ve. Las claves estaban completas, sin mezcla de
  idiomas. Requiere saber quién ve esa pantalla y cuándo.
- **Defensa:** `PRODUCT_CONTRACTS.md` (textos PROHIBIDOS por estado) + `product_first_contact` escenario B.
- **Riesgo que queda:** la primera vez que un texto es absurdo lo tiene que ver una persona; una vez escrito
  como prohibido, no vuelve.

### 4. La app mostraba v0.4.0 corriendo 1.0.2
- **Invariante violada:** la versión sale de UNA fuente: el propio build.
- **Por qué no se detectó:** tres causas a la vez — `tauri.conf.json` y `Cargo.toml` desincronizados
  (`app_version()` lee `CARGO_PKG_VERSION`), un `<span>v0.4.0</span>` a mano como respaldo, y `checkForUpdate`
  leyendo la versión DEL DOM.
- **Defensa:** candado en el CI (tauri.conf == Cargo.toml == tag del release) + cero versiones a mano +
  `runningVersion` como única fuente.
- **Control negativo:** probado con un tag falso (v1.0.9) → falla.

### 5. Publiqué la 1.0.3 y rompí el actualizador de TODOS
- **Invariante violada:** toda versión pública tiene que ser detectable por la versión anterior.
- **Por qué no se detectó:** el `latest.json` lo generaba yo A MANO. Nada se veía mal: la página perfecta, los
  instaladores bajaban, la API decía "uploaded". Sólo la URL real mostraba el 404. **Leer los metadatos de
  GitHub fue lo que me engañó.**
- **Defensa:** `.github/workflows/publish-manifest.yml` (se genera solo al publicar) + `tools/check_updater.py`
  (consulta la URL pública, como la app).
- **Control negativo:** probado pidiendo una versión que no existe → falla.

### 6. La batería de 10.000 billeteras llevaba HORAS en rojo
- **Invariante violada:** una suite tiene que demostrar que ejecutó algo.
- **Por qué no se detectó:** ni siquiera arrancaba (moría en el build script). Nadie miraba un workflow que se
  había puesto rojo en silencio, y "los tests pasan" era algo **creído, no chequeado**. Estuvo roto en 3
  versiones ya publicadas.
- **Defensa:** `tools/run_tests_verified.py` (mínimo de tests + cero fallidos + no sospechosamente rápido).
- **Control negativo:** reproducido — `cargo test` con un filtro que no matchea sale **VERDE habiendo probado
  nada**.

### 7. Rompí la base del nodo en la máquina de Fernando, DOS veces
- **Invariante violada:** usar el entorno más barato disponible; cierre ordenado antes de forzar.
- **Por qué no se detectó:** no había nada que lo impidiera. Maté el proceso con `Stop-Process -Force`.
- **Defensa:** la auto-reparación (`classify_failure` + causalidad por log del intento actual) + Regla 5.
- **Efecto lateral bueno:** el bug me obligó a escribir la reparación, que después funcionó en la vida real.

### 8. La marca anti-bucle no se borraba tras una reparación exitosa
- **Invariante violada:** un mecanismo de protección no puede dejar peor al usuario que no tenerlo.
- **Por qué no se detectó:** la reparación FUNCIONÓ. El bug sólo aparecía la **segunda** vez (cada
  actualización cierra la app a la fuerza, que es lo que corrompe la base). Los tests estaban verdes.
- **Cómo se encontró:** fui a mirar la máquina de Fernando después de que actualizara, en vez de asumir.
- **Defensa:** marca con fecha (reciente = bucle, vieja = incidente cerrado) + 4 tests con esa secuencia.

### 9. Le abrí ventanas de prueba encima de una partida
- **Invariante violada:** jerarquía de entornos (runner > temporal > desarrollo > máquina de Fernando).
- **Por qué no se detectó:** ninguna regla lo prohibía. Y **el mismo probador ya corría en la nube**, donde
  había pasado esa misma mañana.
- **Defensa:** Regla 5. El E2E corre en el runner de Windows, nunca local.

### 10. Intenté copiar un archivo de credenciales a un repo PÚBLICO
- **Invariante violada:** los secretos jamás entran a un árbol Git público, ni temporalmente.
- **Por qué no se detectó:** lo frenó un clasificador externo, no yo. **La operación nunca debió llegar ahí.**
- **Defensa:** Regla 5 + `.gitignore` estricto (client_secret, token, gsc_auth, *.pem, *.key) + apuntar los
  scripts a `C:\secure\fernando-secrets` en vez de copiar.
- **Riesgo que queda:** la separación depende del .gitignore. Un archivo con nombre nuevo se colaría.

---

## El patrón detrás de los 10

ChatGPT los agrupó en **5 causas, no 10 problemas** (ROOT_CAUSE 2026-07-14):

- **A. Nadie era dueño del producto completo** → incidentes 1, 2, 3, 4. Los agentes revisaron piezas; ninguno
  recorrió "soy una persona que abre Brisvia por primera vez".
- **B. Evidencia no observada ni exigida** → 5, 6. Verde/publicado aceptado sin exigir prueba del efecto.
- **C. Destructivo sin aislamiento** → 7, 9. Actué sobre el entorno más valioso teniendo entornos más baratos.
- **D. Límites de seguridad fuera del flujo normal** → 10.
- **E. Diagnóstico por intuición antes de reproducir** → 1.

> *"Convertís ausencia de error visible en evidencia de éxito."*
>
> El workflow existía → asumí que probaba. El release tenía archivos → asumí que actualizaba. El código
> compilaba → asumí que mostraba la versión correcta. La reparación terminó → asumí que el estado quedó bien.
