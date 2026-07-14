//! Byte-exact block header reconstruction for pool (stratum) mining.
//!
//! When mining on a pool, the miner must rebuild the EXACT same 80-byte header the pool re-derives on
//! its side (see the pool's `block_builder.py::build_coinbase_and_header`). If a single byte differs the
//! RandomX hash differs and every share is rejected. This module mirrors the pool's serialization
//! byte-for-byte: BIP34 height push, coinbase (payout output + segwit witness commitment), merkle root
//! over `[coinbase_txid, other_txids]`, and the final header. It is pure (no RandomX, no I/O) so it can
//! be unit-tested against a reference vector produced by the pool's own Python code.
//!
//! Byte order note: txids and the merkle root are handled in *internal* byte order (as Bitcoin serializes
//! them on the wire); the template's `previousblockhash` is in display order, so it is reversed here.

use bitcoin::hashes::{sha256d, Hash};
use serde_json::Value;

/// One transaction as it appears in `getblocktemplate.transactions`.
#[derive(Clone, Debug)]
pub struct TemplateTx {
    /// Display-order txid (as the node returns it in the template).
    pub txid: String,
    /// Full serialized transaction hex (used to assemble the block body; not needed for the header).
    pub data: String,
}

/// The subset of a `getblocktemplate` result needed to rebuild the coinbase + header.
#[derive(Clone, Debug)]
pub struct Template {
    pub version: i32,
    pub previousblockhash: String,
    pub curtime: u32,
    /// Compact target (nBits) as a 4-byte hex string, e.g. "1e00ffff".
    pub bits: String,
    pub height: u64,
    pub coinbasevalue: u64,
    /// Full witness-commitment scriptPubKey hex (starts with 6a24aa21a9ed...).
    pub default_witness_commitment: String,
    pub transactions: Vec<TemplateTx>,
}

impl Template {
    /// Parse the fields we need from a raw `getblocktemplate` JSON result. Returns None if a mandatory
    /// field is missing or malformed (defensive: never panics on unexpected node output).
    pub fn from_template(v: &Value) -> Option<Template> {
        let version = v.get("version")?.as_i64()? as i32;
        let previousblockhash = v.get("previousblockhash")?.as_str()?.to_string();
        let curtime = v.get("curtime")?.as_u64()? as u32;
        let bits = v.get("bits")?.as_str()?.to_string();
        let height = v.get("height")?.as_u64()?;
        let coinbasevalue = v.get("coinbasevalue")?.as_u64()?;
        // A segwit template always carries a witness commitment; without it we cannot build the coinbase.
        let default_witness_commitment = v.get("default_witness_commitment")?.as_str()?.to_string();
        let mut transactions = Vec::new();
        if let Some(arr) = v.get("transactions").and_then(|t| t.as_array()) {
            for t in arr {
                let txid = match t.get("txid").and_then(|x| x.as_str()) {
                    Some(s) => s.to_string(),
                    None => return None,
                };
                let data = t.get("data").and_then(|x| x.as_str()).unwrap_or("").to_string();
                transactions.push(TemplateTx { txid, data });
            }
        }
        Some(Template { version, previousblockhash, curtime, bits, height, coinbasevalue,
                        default_witness_commitment, transactions })
    }
}

/// Double SHA-256 (Bitcoin's hash256), internal byte order.
fn dsha256(bytes: &[u8]) -> [u8; 32] {
    sha256d::Hash::hash(bytes).to_byte_array()
}

/// Bitcoin varint (CompactSize) encoding.
fn varint(n: u64) -> Vec<u8> {
    if n < 0xfd {
        vec![n as u8]
    } else if n <= 0xffff {
        let mut v = vec![0xfd];
        v.extend_from_slice(&(n as u16).to_le_bytes());
        v
    } else if n <= 0xffff_ffff {
        let mut v = vec![0xfe];
        v.extend_from_slice(&(n as u32).to_le_bytes());
        v
    } else {
        let mut v = vec![0xff];
        v.extend_from_slice(&n.to_le_bytes());
        v
    }
}

/// BIP34 block-height push, byte-identical to the node's `CScript() << nHeight` (see the pool's
/// `encode_bip34_height`). Returns the complete scriptSig fragment (its own push opcode included).
///  - 0            -> OP_0            (0x00)
///  - 1..=16       -> OP_1..OP_16     (0x51..0x60)   (compact opcodes, NOT a pushdata)
///  - >16          -> pushdata of the minimal CScriptNum body (len prefix + little-endian body)
fn encode_bip34_height(n: u64) -> Vec<u8> {
    if n == 0 {
        return vec![0x00];
    }
    if (1..=16).contains(&n) {
        return vec![0x50 + n as u8];
    }
    let mut body: Vec<u8> = Vec::new();
    let mut an = n;
    while an != 0 {
        body.push((an & 0xff) as u8);
        an >>= 8;
    }
    // Heights are positive, so if the top bit of the last byte is set, append a 0x00 sign byte.
    if body.last().map(|b| b & 0x80 != 0).unwrap_or(false) {
        body.push(0x00);
    }
    let mut out = vec![body.len() as u8];
    out.extend_from_slice(&body);
    out
}

/// Serialize the segwit coinbase transaction. Returns (full_witness_tx_bytes, txid_internal_bytes).
/// Mirrors the pool's `build_coinbase`.
fn build_coinbase(height: u64, payout_spk: &[u8], value: u64, witness_commitment: &[u8],
                  extranonce: &[u8]) -> (Vec<u8>, [u8; 32]) {
    // scriptSig = BIP34 height (complete fragment) + push(extranonce)
    let mut script_sig = encode_bip34_height(height);
    script_sig.extend_from_slice(&varint(extranonce.len() as u64));
    script_sig.extend_from_slice(extranonce);

    // input: null prevout (32 zero bytes + 0xffffffff index), our scriptSig, sequence 0xffffffff
    let mut tx_in = Vec::new();
    tx_in.extend_from_slice(&[0u8; 32]);
    tx_in.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    tx_in.extend_from_slice(&varint(script_sig.len() as u64));
    tx_in.extend_from_slice(&script_sig);
    tx_in.extend_from_slice(&0xffff_ffffu32.to_le_bytes());

    // outputs: [0] payout to the pool, [1] witness commitment (OP_RETURN)
    let mut outputs = varint(2);
    outputs.extend_from_slice(&(value as i64).to_le_bytes());
    outputs.extend_from_slice(&varint(payout_spk.len() as u64));
    outputs.extend_from_slice(payout_spk);
    outputs.extend_from_slice(&0i64.to_le_bytes());
    outputs.extend_from_slice(&varint(witness_commitment.len() as u64));
    outputs.extend_from_slice(witness_commitment);

    // non-witness serialization -> txid
    let mut nowit = Vec::new();
    nowit.extend_from_slice(&2u32.to_le_bytes());
    nowit.extend_from_slice(&varint(1));
    nowit.extend_from_slice(&tx_in);
    nowit.extend_from_slice(&outputs);
    nowit.extend_from_slice(&0u32.to_le_bytes());
    let txid = dsha256(&nowit);

    // witness serialization -> block body (single 32-byte zero witness, BIP141)
    let mut full = Vec::new();
    full.extend_from_slice(&2u32.to_le_bytes());
    full.extend_from_slice(&[0x00, 0x01]); // segwit marker + flag
    full.extend_from_slice(&varint(1));
    full.extend_from_slice(&tx_in);
    full.extend_from_slice(&outputs);
    full.extend_from_slice(&varint(1)); // witness stack items
    full.extend_from_slice(&varint(32));
    full.extend_from_slice(&[0u8; 32]);
    full.extend_from_slice(&0u32.to_le_bytes());
    (full, txid)
}

/// Merkle root from a list of txids (internal byte order). Mirrors the pool's `merkle_root`.
fn merkle_root(txids: &[[u8; 32]]) -> [u8; 32] {
    if txids.is_empty() {
        return [0u8; 32];
    }
    let mut layer: Vec<[u8; 32]> = txids.to_vec();
    while layer.len() > 1 {
        if layer.len() % 2 == 1 {
            let last = *layer.last().unwrap();
            layer.push(last);
        }
        let mut next = Vec::with_capacity(layer.len() / 2);
        let mut i = 0;
        while i < layer.len() {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&layer[i]);
            buf[32..].copy_from_slice(&layer[i + 1]);
            next.push(dsha256(&buf));
            i += 2;
        }
        layer = next;
    }
    layer[0]
}

/// Decode a 32-byte hex string into a fixed array.
fn hex32(s: &str) -> Option<[u8; 32]> {
    let b = hex::decode(s).ok()?;
    if b.len() != 32 {
        return None;
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&b);
    Some(a)
}

/// Build the 80-byte header (with nonce = `nonce`) exactly like the pool's `build_coinbase_and_header`.
/// Returns None if any template field is malformed. `extranonce` = extranonce1 (from the pool) + the
/// extranonce2 chosen by the miner.
pub fn build_header(tpl: &Template, payout_spk: &[u8], nonce: u32, extranonce: &[u8], ntime: u32)
    -> Option<[u8; 80]> {
    let wc = hex::decode(&tpl.default_witness_commitment).ok()?;
    let (_cb_full, cb_txid) = build_coinbase(tpl.height, payout_spk, tpl.coinbasevalue, &wc, extranonce);

    let mut txids: Vec<[u8; 32]> = Vec::with_capacity(1 + tpl.transactions.len());
    txids.push(cb_txid);
    for t in &tpl.transactions {
        // Template txids are in display order; convert to internal (reversed) for the merkle tree.
        let mut id = hex32(&t.txid)?;
        id.reverse();
        txids.push(id);
    }
    let mroot = merkle_root(&txids);

    let mut prev = hex32(&tpl.previousblockhash)?;
    prev.reverse(); // display -> internal

    let bits = u32::from_str_radix(&tpl.bits, 16).ok()?;

    let mut header = [0u8; 80];
    header[0..4].copy_from_slice(&(tpl.version as u32).to_le_bytes());
    header[4..36].copy_from_slice(&prev);
    header[36..68].copy_from_slice(&mroot);
    header[68..72].copy_from_slice(&ntime.to_le_bytes());
    header[72..76].copy_from_slice(&bits.to_le_bytes());
    header[76..80].copy_from_slice(&nonce.to_le_bytes());
    Some(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_template() -> Template {
        Template {
            version: 536870912,
            previousblockhash:
                "00000000000000000002a7c4c1e48d76c5a37902165a9b3a4b3f4b3f4b3f4b3f".to_string(),
            curtime: 1750000000,
            bits: "1e00ffff".to_string(),
            height: 12345,
            coinbasevalue: 5000000000,
            default_witness_commitment: format!("6a24aa21a9ed{}", "ab".repeat(32)),
            transactions: vec![],
        }
    }

    // Reference vectors produced by the pool's own block_builder.py (pure functions) for a synthetic
    // empty-mempool template. If these ever diverge, pool mining would silently produce rejected shares.
    const EN1: &str = "deadbeef";
    const EN2: &str = "0000000000000001";
    const NONCE: u32 = 0x12345678;
    const NTIME: u32 = 1750000123;

    fn extranonce() -> Vec<u8> {
        let mut e = hex::decode(EN1).unwrap();
        e.extend_from_slice(&hex::decode(EN2).unwrap());
        e
    }

    #[test]
    fn header_matches_pool_reference_no_tx() {
        let tpl = sample_template();
        let spk = hex::decode(format!("0014{}", "11".repeat(20))).unwrap();
        let h = build_header(&tpl, &spk, NONCE, &extranonce(), NTIME).unwrap();
        let expected = "000000203f4b3f4b3f4b3f4b3a9b5a160279a3c5768de4c1c4a7020000000000000000000c82c34866579ccb6bafbb6d471a9cda3680bf90dcaf59775bb6a0012a4e0f8afbe14e68ffff001e78563412";
        assert_eq!(hex::encode(h), expected);
    }

    #[test]
    fn header_matches_pool_reference_bip34_opn_height() {
        // Height 7 exercises the OP_1..OP_16 branch of the BIP34 height push.
        let mut tpl = sample_template();
        tpl.height = 7;
        let spk = hex::decode(format!("0014{}", "11".repeat(20))).unwrap();
        let h = build_header(&tpl, &spk, NONCE, &extranonce(), NTIME).unwrap();
        let expected = "000000203f4b3f4b3f4b3f4b3a9b5a160279a3c5768de4c1c4a7020000000000000000006a2acc3dd46b837378050d4cacc09de87add11740e0b230d1a76e989162c1a2afbe14e68ffff001e78563412";
        assert_eq!(hex::encode(h), expected);
    }

    #[test]
    fn header_matches_pool_reference_with_one_tx() {
        // One extra transaction exercises the merkle pairing [coinbase, tx].
        let mut tpl = sample_template();
        tpl.transactions = vec![TemplateTx { txid: "cd".repeat(32), data: "00".to_string() }];
        let spk = hex::decode(format!("0014{}", "11".repeat(20))).unwrap();
        let h = build_header(&tpl, &spk, NONCE, &extranonce(), NTIME).unwrap();
        let expected = "000000203f4b3f4b3f4b3f4b3a9b5a160279a3c5768de4c1c4a702000000000000000000d12e74ae7fd1737cbc6f6c9ce5b7aab3cc7784bc4b367b881b3381e99953d1ccfbe14e68ffff001e78563412";
        assert_eq!(hex::encode(h), expected);
    }

    #[test]
    fn varint_boundaries() {
        assert_eq!(varint(0xfc), vec![0xfc]);
        assert_eq!(varint(0xfd), vec![0xfd, 0xfd, 0x00]);
        assert_eq!(varint(0x1234), vec![0xfd, 0x34, 0x12]);
        assert_eq!(varint(0x00010000), vec![0xfe, 0x00, 0x00, 0x01, 0x00]);
    }

    #[test]
    fn bip34_height_encodings() {
        assert_eq!(encode_bip34_height(0), vec![0x00]);
        assert_eq!(encode_bip34_height(1), vec![0x51]);
        assert_eq!(encode_bip34_height(16), vec![0x60]);
        assert_eq!(encode_bip34_height(17), vec![0x01, 0x11]);
        assert_eq!(encode_bip34_height(12345), vec![0x02, 0x39, 0x30]);
        // 0x80 top bit set on the last byte -> sign byte appended.
        assert_eq!(encode_bip34_height(128), vec![0x02, 0x80, 0x00]);
    }
}
