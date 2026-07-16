#!/usr/bin/env python3
"""Cryptographically verify updater bundles against their minisign signatures.

check_updater.py proves the live manifest is reachable and its signature field is non-empty, but it
never checks that the signature actually signs the bytes users download. This runs the real
cryptography the app runs before it installs an update, against the exact artifacts:

  - the Ed25519 signature over blake2b-512(bundle) is valid under the PRODUCTION public key
    (read straight from src-tauri/tauri.conf.json -- the same key baked into every installed copy);
  - the key id in the signature matches the public key;
  - the trusted comment ("file:...") is itself signed and names this bundle;
  - a bundle altered by one byte is REJECTED;
  - a corrupted signature is REJECTED;
  - the wrong public key is REJECTED.

Tauri emits prehashed minisign signatures (algorithm "ED"): the signed message is blake2b-512 of the
file, not the raw file. The public key line stores "Ed"; only the key id (bytes 2..10) must match.

Usage:
  python tools/verify_updater_signature.py [dir]     verify every <bundle>+<bundle>.sig pair under dir
  python tools/verify_updater_signature.py <bundle> <bundle>.sig
Exit 0 = every check (positive and negative) held for every bundle. Exit 1 = a check failed.
"""
import base64
import glob
import hashlib
import json
import os
import sys

from nacl.exceptions import BadSignatureError
from nacl.signing import VerifyKey

HERE = os.path.dirname(os.path.abspath(__file__))
TAURI_CONF = os.path.join(HERE, "..", "src-tauri", "tauri.conf.json")


def production_pubkey():
    with open(TAURI_CONF, encoding="utf-8") as f:
        return json.load(f)["plugins"]["updater"]["pubkey"]


def _parse_pubkey(pubkey_field):
    inner = base64.b64decode(pubkey_field).decode()
    raw = base64.b64decode(inner.strip().splitlines()[-1])  # 2 alg + 8 key id + 32 public key
    return raw[2:10], raw[10:42]


def _parse_sig(sig_text):
    lines = base64.b64decode(sig_text).decode().splitlines()
    sig_raw = base64.b64decode(lines[1])       # 2 alg + 8 key id + 64 signature
    alg, key_id, signature = sig_raw[:2], sig_raw[2:10], sig_raw[10:74]
    trusted_comment = lines[2].split("trusted comment: ", 1)[1]
    global_sig = base64.b64decode(lines[3])    # signature over signature||trusted_comment
    return alg, key_id, signature, trusted_comment, global_sig


def _ok(pubkey32, message, signature):
    try:
        VerifyKey(pubkey32).verify(message, signature)
        return True
    except BadSignatureError:
        return False


def verify_bundle(pubkey_field, bundle_bytes, sig_text, expected_name=None):
    reasons = []
    pk_key_id, pubkey32 = _parse_pubkey(pubkey_field)
    alg, sig_key_id, signature, trusted_comment, global_sig = _parse_sig(sig_text)

    if alg != b"ED":
        reasons.append(f"unexpected signature algorithm {alg!r} (Tauri prehashed signatures are 'ED')")
    if sig_key_id != pk_key_id:
        reasons.append("key id in the signature does not match the public key: signed by a different key")
        return False, reasons

    if not _ok(pubkey32, hashlib.blake2b(bundle_bytes, digest_size=64).digest(), signature):
        reasons.append("the signature does NOT sign these bundle bytes")
    if not _ok(pubkey32, signature + trusted_comment.encode(), global_sig):
        reasons.append("the trusted comment is not signed (it could have been tampered)")

    if expected_name:
        named = trusted_comment.split("file:", 1)[1] if "file:" in trusted_comment else ""
        # GitHub rewrites spaces to dots in uploaded asset names; the trusted comment keeps the
        # original productName ("Brisvia Miner..."). Compare with that rewrite applied.
        if named != expected_name and named.replace(" ", ".") != expected_name:
            reasons.append(f"the trusted comment names '{named}', not '{expected_name}'")

    return (not reasons), reasons


def check_one(pubkey_field, bundle_path, sig_path):
    with open(bundle_path, "rb") as f:
        bundle = f.read()
    with open(sig_path) as f:
        sig = f.read().strip()
    name = os.path.basename(bundle_path)
    print(f"  {name} ({len(bundle):,} bytes)")

    ok, why = verify_bundle(pubkey_field, bundle, sig, name)
    if not ok:
        for r in why:
            print("    - " + r)
        return False
    print("    positive: authentic bundle accepted under the production key")

    tampered = bytearray(bundle)
    tampered[len(tampered) // 2] ^= 0x01
    if verify_bundle(pubkey_field, bytes(tampered), sig, name)[0]:
        print("    - a TAMPERED bundle was accepted")
        return False
    print("    negative: one flipped byte -> rejected")

    lines = base64.b64decode(sig).decode().splitlines()
    raw = bytearray(base64.b64decode(lines[1]))
    raw[40] ^= 0x01
    lines[1] = base64.b64encode(bytes(raw)).decode()
    bad_sig = base64.b64encode("\n".join(lines).encode() + b"\n").decode()
    if verify_bundle(pubkey_field, bundle, bad_sig, name)[0]:
        print("    - a CORRUPTED signature was accepted")
        return False
    print("    negative: corrupted signature -> rejected")

    inner = base64.b64decode(pubkey_field).decode().splitlines()
    kraw = bytearray(base64.b64decode(inner[-1]))
    kraw[20] ^= 0x01
    inner[-1] = base64.b64encode(bytes(kraw)).decode()
    wrong_key = base64.b64encode("\n".join(inner).encode()).decode()
    if verify_bundle(wrong_key, bundle, sig, name)[0]:
        print("    - a bundle verified under the WRONG key")
        return False
    print("    negative: wrong public key -> rejected")
    return True


def main(argv):
    pubkey_field = production_pubkey()
    pairs = []
    if len(argv) == 2 and os.path.isfile(argv[0]) and os.path.isfile(argv[1]):
        pairs = [(argv[0], argv[1])]
    else:
        base = argv[0] if argv else "."
        for sig in sorted(glob.glob(os.path.join(base, "*.sig"))):
            bundle = sig[:-4]
            if os.path.isfile(bundle):
                pairs.append((bundle, sig))
    if not pairs:
        sys.exit("no <bundle>+<bundle>.sig pairs found: pass a bundle and its .sig, or a directory")

    print(f"verifying {len(pairs)} updater bundle(s) against the production minisign key")
    all_ok = all(check_one(pubkey_field, b, s) for b, s in pairs)
    if not all_ok:
        sys.exit("\nUPDATER SIGNATURE VERIFICATION FAILED")
    print(f"\nOK: {len(pairs)} bundle(s) authentic; tampering, bad signatures and wrong keys all rejected.")


if __name__ == "__main__":
    main(sys.argv[1:])
