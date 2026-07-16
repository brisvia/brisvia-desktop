"""Apply one ablation to a checked-out lib.rs. One variable removed per variant, nothing else.

WHY A LADDER AND NOT A GUESS
----------------------------
rc6's node_shutdown_tests build a whole AppState, and AppState carries
`tray: Arc<Mutex<Option<tauri::tray::TrayIcon>>>`. Even holding None, the TYPE has to be linked, which
drags Tauri's runtime into the test executable. That is a plausible route to 0xC0000139 -- Windows started
a DLL and could not find a procedure in it.

Plausible is not proven. Each variant removes exactly one thing, so whichever one loads names the cause:

  rc5-control            rc5 as it shipped. The half that already loads. Proves the harness, not the fix.
  rc6-intact             rc6 untouched. Reproduces the failure, or there is nothing here to find.
  rc6-no-shutdown-tests  the whole module deleted.       loads -> the module is the cause
  rc6-no-appstate        module kept, AppState construction gone.  loads -> AppState is the cause,
                                                          not the module, and not the tests
  rc6-minimal-helper     tests kept, on a real Child and a decoupled helper.  loads -> that is the fix

If 3 loads but 4 does not, the cause is not what I think it is, and the ladder says so instead of me.

NOT A PRODUCT CHANGE
--------------------
These patches are applied to a checkout inside the runner and are never committed. The product changes only
after a sealed run says which variant removes the failure -- and then it changes deliberately, once.

    python ablate.py --variant rc6-no-appstate --file src-tauri/src/lib.rs
    python ablate.py --self-test
"""
import argparse
import hashlib
import pathlib
import re
import subprocess
import sys

VARIANTES = ("rc5-control", "rc6-intact", "rc6-no-shutdown-tests", "rc6-no-appstate",
             "rc6-minimal-helper")


class AblacionFallida(Exception):
    """The patch did not do what it claims. Running a variant that is not the variant proves nothing."""


def _bloque(texto, cabecera):
    """Span of a brace-balanced block starting at `cabecera`. Returns (start, end) or raises.

    Counting braces and not regex: a regex cannot balance, and 'the next line that starts with }' guesses.
    Strings and comments containing braces would fool this; the module it is pointed at has none, and the
    self-test asserts the extracted span is exactly the module.
    """
    i = texto.find(cabecera)
    if i < 0:
        raise AblacionFallida(f"no encontre '{cabecera}'")
    j = texto.find("{", i)
    if j < 0:
        raise AblacionFallida(f"'{cabecera}' no abre llave")
    prof = 0
    for k in range(j, len(texto)):
        if texto[k] == "{":
            prof += 1
        elif texto[k] == "}":
            prof -= 1
            if prof == 0:
                return i, k + 1
    raise AblacionFallida(f"'{cabecera}' no cierra")


def sin_modulo_de_cierre(texto):
    """Delete mod node_shutdown_tests entirely, and the #[cfg(test)] that introduces it."""
    ini, fin = _bloque(texto, "mod node_shutdown_tests")
    # The attribute lines above the mod: #[cfg(test)] and any #[allow(...)].
    cabeza = texto.rfind("\n#[cfg(test)]", 0, ini)
    if cabeza < 0 or ini - cabeza > 200:
        cabeza = ini
    return texto[:cabeza] + "\n" + texto[fin:]


def sin_appstate(texto):
    """Keep the module. Remove AppState construction and everything that needs it.

    Leaves only the test that reads NODE_SHUTDOWN_MAX, which touches no Tauri type at all. If the binary
    loads now, AppState is the cause and the tests are innocent.
    """
    ini, fin = _bloque(texto, "mod node_shutdown_tests")
    nuevo = '''mod node_shutdown_tests {
    use super::*;

    // ABLATION rc6-no-appstate: estado_con() built a whole AppState, and AppState carries
    // tray: Arc<Mutex<Option<tauri::tray::TrayIcon>>>. Even as None the type must be linked. This
    // variant keeps the module and removes only that. Nothing here touches a Tauri type.
    #[test]
    fn el_plazo_es_de_180_segundos_no_de_cinco() {
        assert_eq!(NODE_SHUTDOWN_MAX, Duration::from_secs(180));
    }
}'''
    return texto[:ini] + nuevo + texto[fin:]


def helper_minimo(texto):
    """The tests keep testing shutdown -- against a real Child and a helper that never sees AppState.

    This is the shape the fix would take: the helper takes the slot and the timeout, the product wraps it
    and pulls the slot out of AppState. If this loads AND the tests pass, the fix is proven before a line
    of product code moves.
    """
    ini, fin = _bloque(texto, "mod node_shutdown_tests")
    nuevo = '''mod node_shutdown_tests {
    use super::*;
    use std::process::Child;

    // ABLATION rc6-minimal-helper: the shape the fix would take. The helper is handed the process slot
    // and the timeout and nothing else -- no AppState, no TrayIcon, no Tauri runtime. The product would
    // keep a wrapper that pulls the slot out of AppState, so nothing at the call sites changes.
    fn esperar_cierre(ranura: &Arc<Mutex<Option<Child>>>, plazo: Duration) -> bool {
        let inicio = std::time::Instant::now();
        loop {
            {
                let mut g = ranura.lock().unwrap();
                match g.as_mut() {
                    None => return true,
                    Some(c) => {
                        if let Ok(Some(_)) = c.try_wait() {
                            *g = None;
                            return true;
                        }
                    }
                }
            }
            if inicio.elapsed() >= plazo {
                return false;  // never killed: that was the defect
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn proceso_terco() -> Child {
        #[cfg(target_os = "windows")]
        let c = std::process::Command::new("cmd").args(["/c", "ping -n 60 127.0.0.1 >nul"]).spawn();
        #[cfg(not(target_os = "windows"))]
        let c = std::process::Command::new("sleep").arg("60").spawn();
        c.expect("no se pudo lanzar el proceso de prueba")
    }

    #[test]
    fn el_plazo_es_de_180_segundos_no_de_cinco() {
        assert_eq!(NODE_SHUTDOWN_MAX, Duration::from_secs(180));
    }

    #[test]
    fn si_el_nodo_no_cierra_devuelve_false_y_NO_lo_mata() {
        let terco = proceso_terco();
        let pid = terco.id();
        let ranura = Arc::new(Mutex::new(Some(terco)));
        assert!(!esperar_cierre(&ranura, Duration::from_millis(300)));
        let mut g = ranura.lock().unwrap();
        let c = g.as_mut().expect("lo mato, y no debia");
        assert!(matches!(c.try_wait(), Ok(None)), "sigue vivo: pid {}", pid);
        let _ = c.kill();
    }

    #[test]
    fn si_el_nodo_ya_cerro_devuelve_true() {
        let ranura: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        assert!(esperar_cierre(&ranura, Duration::from_millis(50)));
    }
}'''
    return texto[:ini] + nuevo + texto[fin:]


PARCHES = {
    "rc5-control": None,
    "rc6-intact": None,
    "rc6-no-shutdown-tests": sin_modulo_de_cierre,
    "rc6-no-appstate": sin_appstate,
    "rc6-minimal-helper": helper_minimo,
}


def aplicar(variante, ruta):
    texto = ruta.read_text(encoding="utf-8")
    antes = hashlib.sha256(texto.encode()).hexdigest()
    fn = PARCHES[variante]
    if fn is None:
        print(f"  {variante}: sin parche, el fuente queda intacto")
        print(f"  sha256 {antes}")
        return antes

    nuevo = fn(texto)
    if nuevo == texto:
        raise AblacionFallida(f"{variante}: el parche no cambio nada")
    if "mod node_shutdown_tests" in nuevo and variante == "rc6-no-shutdown-tests":
        raise AblacionFallida("rc6-no-shutdown-tests: el modulo sigue ahi")
    ruta.write_text(nuevo, encoding="utf-8")
    despues = hashlib.sha256(nuevo.encode()).hexdigest()
    print(f"  {variante}: aplicado")
    print(f"  sha256 {antes[:16]} -> {despues}")
    return despues


def self_test():
    """Every patch, against the REAL rc6 source. A patch validated on a toy proves nothing."""
    fallos = 0
    r = subprocess.run(["git", "show", "daf47da0:src-tauri/src/lib.rs"],
                       cwd=r"C:/dev/brisvia-miner", capture_output=True, text=True, encoding="utf-8")
    real = r.stdout
    if not real or "mod node_shutdown_tests" not in real:
        print("  FAIL  no pude leer el lib.rs real de rc6")
        return 1
    print(f"  lib.rs de rc6: {len(real.splitlines())} lineas")

    ini, fin = _bloque(real, "mod node_shutdown_tests")
    modulo = real[ini:fin]
    if not modulo.startswith("mod node_shutdown_tests {") or not modulo.endswith("}"):
        print("  FAIL  el extractor de bloques no agarro el modulo entero")
        fallos += 1
    elif modulo.count("{") != modulo.count("}"):
        print("  FAIL  el bloque extraido no esta balanceado")
        fallos += 1
    else:
        print(f"  PASS  extrae-el-modulo-entero-balanceado  ({len(modulo.splitlines())} lineas)")

    # The thing under suspicion is actually in there. If it is not, the whole hypothesis is wrong and the
    # ladder would be testing nothing.
    if "tray:" not in modulo or "AppState {" not in modulo:
        print("  FAIL  el modulo no construye AppState: la hipotesis no se sostiene")
        fallos += 1
    else:
        print("  PASS  el-modulo-si-construye-AppState-con-tray")

    for v, comprueba in (
            ("rc6-no-shutdown-tests",
             lambda t: "mod node_shutdown_tests" not in t),
            ("rc6-no-appstate",
             lambda t: "mod node_shutdown_tests" in t and "AppState {" not in t.split(
                 "mod node_shutdown_tests")[1]),
            ("rc6-minimal-helper",
             lambda t: "mod node_shutdown_tests" in t and "esperar_cierre" in t and "AppState {" not in
                       t.split("mod node_shutdown_tests")[1])):
        try:
            salida = PARCHES[v](real)
        except AblacionFallida as e:
            print(f"  FAIL  {v}: {e}")
            fallos += 1
            continue
        if salida == real:
            print(f"  FAIL  {v}: no cambio nada")
            fallos += 1
        elif not comprueba(salida):
            print(f"  FAIL  {v}: no hizo lo que dice hacer")
            fallos += 1
        elif salida.count("{") != salida.count("}"):
            print(f"  FAIL  {v}: dejo las llaves desbalanceadas -- no compilaria")
            fallos += 1
        else:
            # Spelled out, not signed: "+92" reads as "added 92" when it means the opposite.
            quitadas = len(real.splitlines()) - len(salida.splitlines())
            print(f"  PASS  {v}  (quita {quitadas} lineas)")

    # Each variant must remove something DIFFERENT. Two identical variants are one variant and a wasted
    # build.
    hechas = {v: PARCHES[v](real) for v in ("rc6-no-shutdown-tests", "rc6-no-appstate",
                                            "rc6-minimal-helper")}
    if len(set(hechas.values())) != 3:
        print("  FAIL  dos variantes producen el mismo fuente: una de las dos no prueba nada")
        fallos += 1
    else:
        print("  PASS  las-tres-variantes-son-distintas-entre-si")

    print()
    if fallos:
        print(f"SELF-TEST FAILED ({fallos})")
        return 1
    print("self-test OK: cada parche hace lo que dice, sobre el fuente real de rc6")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--variant", choices=VARIANTES)
    ap.add_argument("--file", default="src-tauri/src/lib.rs")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.variant:
        return ap.error("--variant es obligatorio")
    ruta = pathlib.Path(a.file)
    if not ruta.exists():
        print(f"no existe: {ruta}")
        return 1
    try:
        aplicar(a.variant, ruta)
    except AblacionFallida as e:
        print(f"ABLACION FALLIDA: {e}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
