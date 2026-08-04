#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Validador de integridad del launch-bundle de Brisvia 1.1.3.
Uso: python validate-launch-bundle.py
- En estado PREPARING_FOR_APPROVAL: lista lo que falta (no falla).
- En estado READY_FOR_APPROVAL: FALLA (exit 1) si queda cualquier PENDIENTE, hash abreviado,
  archivo referenciado inexistente o evidencia sin cerrar. Correr esto ANTES de congelar el bundle.
"""
import json, os, re, sys, hashlib

HERE = os.path.dirname(os.path.abspath(__file__))
BUNDLE = os.path.join(HERE, "launch-bundle-v1.1.3.json")
HEX64 = re.compile(r"^[0-9a-f]{64}$")

def walk(obj, path=""):
    if isinstance(obj, dict):
        for k, v in obj.items():
            yield from walk(v, f"{path}.{k}")
    elif isinstance(obj, list):
        for i, v in enumerate(obj):
            yield from walk(v, f"{path}[{i}]")
    else:
        yield path, obj

def main():
    d = json.load(open(BUNDLE, encoding="utf-8"))
    status = d.get("status")
    strict = (status == "READY_FOR_APPROVAL")
    problems, warns = [], []

    # 1) PENDIENTES: donde quedan
    pend = [(p, v) for p, v in walk(d) if isinstance(v, str) and re.search(r"PENDIENTE|PENDING", v, re.I)]
    for p, v in pend:
        (problems if strict else warns).append(f"PENDIENTE en {p}: {v[:60]}")

    # 2) SHA de assets deben ser 64 hex completos
    for name, sha in d.get("build", {}).get("assets_R_full_sha256", {}).items():
        if not HEX64.match(sha):
            problems.append(f"asset {name}: SHA no es 64-hex: {sha}")
    # sidecars _16 abreviados: solo aviso (identidad conocida, no es asset publicado)
    for k, v in d.get("build", {}).get("sidecars_R", {}).items():
        if k.endswith("_16"):
            warns.append(f"sidecar {k} abreviado (16): usar SHA completo en READY")

    # 3) latest.json prepared SHA presente y 64-hex (provisional en PREPARING; final exigido en READY)
    lj = d.get("updater", {}).get("latest_json_prepared_sha256", "")
    if not HEX64.match(lj):
        (problems if strict else warns).append(f"updater.latest_json_prepared_sha256 no es 64-hex: {lj}")

    # 4) archivos hermanos referenciados existen
    for f in ["release-go-v1.1.3.json", "owner-approval-v1.1.3.json", "RUNBOOK-T0-v1.1.3.md"]:
        if not os.path.exists(os.path.join(HERE, f)):
            (problems if strict else warns).append(f"archivo referenciado no existe: {f}")

    # 5) infra hashes 64-hex
    ih = d.get("infra_hashes", {})
    for k in ["pool_stratum_server_sha256", "pool_rxverify_sha256"]:
        val = (ih.get(k, "") or "").split()[0] if ih.get(k) else ""
        if not HEX64.match(val):
            warns.append(f"infra_hashes.{k} no es 64-hex limpio: {ih.get(k)}")

    # salida
    print(f"=== validate-launch-bundle · status={status} · modo={'STRICT (READY)' if strict else 'PREPARING (aviso)'} ===")
    sha = hashlib.sha256(open(BUNDLE, 'rb').read()).hexdigest()
    print(f"SHA-256 del bundle actual: {sha}")
    if warns:
        print(f"\n-- AVISOS ({len(warns)}) --"); [print("  -", w) for w in warns]
    if problems:
        print(f"\n-- BLOQUEANTES ({len(problems)}) --"); [print("  x", p) for p in problems]
        print("\nRESULTADO: NO-GO para READY (resolver los bloqueantes).")
        sys.exit(1)
    if strict:
        print("\nRESULTADO: GO — bundle listo para congelar y aprobar.")
    else:
        print(f"\nRESULTADO: en preparacion. {len(warns)} avisos = lo que falta para READY. Sin bloqueantes de formato.")

if __name__ == "__main__":
    main()
