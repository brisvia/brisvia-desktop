//! Brisvia miner: CONTINUOUS, MULTITHREADED mining against a node.
//! Each round: getblocktemplate -> build block -> N threads search for a nonce (shared Cache, Vm per thread) ->
//! submitblock -> repeat. Reuses the Cache per seed. Measures real hashrate.
//! Usage: mine-once <rpc_url> <user> <pass> <payout_addr> [max_blocks] [threads] [max_nonces]

use bitcoin::absolute::LockTime;
use bitcoin::blockdata::block::{Block, Header, Version};
use bitcoin::blockdata::script::Builder;
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash;
use bitcoin::transaction::Version as TxVersion;
use bitcoin::{
    Amount, BlockHash, CompactTarget, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxMerkleNode,
    TxOut, Witness,
};
use brisvia_randomx::{Cache, Dataset, Vm};
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// RPC with retries (unstable network). Does not panic; returns Err after exhausting attempts.
fn rpc(url: &str, user: &str, pass: &str, method: &str, params: Value) -> Result<Value, String> {
    let auth = format!("Basic {}", base64(&format!("{user}:{pass}")));
    let mut last = String::new();
    for intento in 0..3 {
        if intento > 0 {
            std::thread::sleep(std::time::Duration::from_millis(600));
        }
        let body = json!({"jsonrpc":"1.0","id":"mine","method":method,"params":params});
        match ureq::post(url).set("Authorization", &auth).send_json(body) {
            Ok(resp) => match resp.into_json::<Value>() {
                Ok(v) => {
                    if !v["error"].is_null() {
                        return Err(format!("RPC {method} error: {}", v["error"]));
                    }
                    return Ok(v["result"].clone());
                }
                Err(e) => last = format!("json {method}: {e}"),
            },
            Err(e) => last = format!("RPC {method}: {e}"),
        }
    }
    Err(last)
}

fn base64(s: &str) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | (*c.get(2).unwrap_or(&0) as u32);
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if c.len() > 1 { T[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if c.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn hex32(s: &str) -> [u8; 32] {
    let b = hex::decode(s).expect("hex");
    let mut a = [0u8; 32];
    a.copy_from_slice(&b);
    a
}

fn build_candidate(tmpl: &Value, payout_script: &ScriptBuf, extranonce: u64) -> (Block, Vec<u8>, [u8; 32], [u8; 32]) {
    let version = tmpl["version"].as_i64().unwrap() as i32;
    let prev = BlockHash::from_str(tmpl["previousblockhash"].as_str().unwrap()).unwrap();
    let curtime = tmpl["curtime"].as_u64().unwrap() as u32;
    let bits = u32::from_str_radix(tmpl["bits"].as_str().unwrap(), 16).unwrap();
    let height = tmpl["height"].as_u64().unwrap();
    let coinbasevalue = tmpl["coinbasevalue"].as_u64().unwrap();
    let target_be = hex32(tmpl["target"].as_str().unwrap());
    let mut seed_key = hex32(tmpl["brisvia"]["randomx_seed_hash"].as_str().unwrap());
    seed_key.reverse();

    let script_sig = Builder::new()
        .push_int(height as i64)
        .push_slice(extranonce.to_le_bytes())
        .push_slice(*b"/Brisvia/")
        .into_script();
    let coinbase_in = TxIn {
        previous_output: OutPoint::null(),
        script_sig,
        sequence: Sequence::MAX,
        witness: Witness::from_slice(&[[0u8; 32]]),
    };
    let mut outs = vec![TxOut { value: Amount::from_sat(coinbasevalue), script_pubkey: payout_script.clone() }];
    if let Some(wc) = tmpl["default_witness_commitment"].as_str() {
        outs.push(TxOut { value: Amount::ZERO, script_pubkey: ScriptBuf::from_hex(wc).unwrap() });
    }
    let coinbase = Transaction {
        version: TxVersion(2),
        lock_time: LockTime::ZERO,
        input: vec![coinbase_in],
        output: outs,
    };

    let mut txdata = vec![coinbase];
    if let Some(txs) = tmpl["transactions"].as_array() {
        for t in txs {
            let raw = hex::decode(t["data"].as_str().unwrap()).unwrap();
            txdata.push(deserialize::<Transaction>(&raw).unwrap());
        }
    }
    let mut block = Block {
        header: Header {
            version: Version::from_consensus(version),
            prev_blockhash: prev,
            merkle_root: TxMerkleNode::all_zeros(),
            time: curtime,
            bits: CompactTarget::from_consensus(bits),
            nonce: 0,
        },
        txdata,
    };
    block.header.merkle_root = block.compute_merkle_root().expect("merkle");
    let header_bytes = serialize(&block.header);
    assert_eq!(header_bytes.len(), 80);
    (block, header_bytes, target_be, seed_key)
}

/// Searches for a valid nonce with `threads` threads (fast/dataset). One thread watches the tip: if a new block
/// appears, it bails out (to avoid mining stale work). Returns (nonce, hashes, stale=bailed out due to new tip).
#[allow(clippy::too_many_arguments)]
fn mine_block(cache: Arc<Cache>, dataset: Arc<Dataset>, header: &[u8], target_be: &[u8; 32], threads: usize,
              max_nonces: u64, url: &str, user: &str, pass: &str, prev_hash: &str, json_mode: bool) -> (Option<u32>, u64, bool) {
    let found = AtomicBool::new(false);
    let stale = AtomicBool::new(false);
    let winner = AtomicU32::new(0);
    let hashes = AtomicU64::new(0);
    std::thread::scope(|s| {
        // Tip watcher: every 1.5s checks whether the best block changed relative to the current template.
        {
            let (found, stale, hashes) = (&found, &stale, &hashes);
            let t_watch = std::time::Instant::now();
            s.spawn(move || {
                let mut ticks = 0u32;
                while !found.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(150)); // responds quickly to "found"
                    ticks += 1;
                    if found.load(Ordering::Relaxed) { break; }
                    // Live hashrate ~every 2s so the UI shows real-time speed, not only when a block is accepted.
                    if json_mode && ticks % 13 == 0 {
                        let hs = hashes.load(Ordering::Relaxed) as f64 / t_watch.elapsed().as_secs_f64().max(0.001);
                        if hs > 0.0 { println!("{}", json!({"event":"hashrate","hashrate":hs})); }
                    }
                    if ticks % 10 == 0 { // queries the tip ~every 1.5s
                        if let Ok(best) = rpc(url, user, pass, "getbestblockhash", json!([])) {
                            if best.as_str() != Some(prev_hash) {
                                stale.store(true, Ordering::SeqCst);
                                found.store(true, Ordering::SeqCst);
                                break;
                            }
                        }
                    }
                }
            });
        }
        for t in 0..threads {
            let (cache, dataset, found, winner, hashes) = (cache.clone(), dataset.clone(), &found, &winner, &hashes);
            s.spawn(move || {
                let vm = Vm::new_fast(cache, dataset);
                let mut local = header.to_vec();
                let mut count = 0u64;
                let mut nonce = t as u64;
                while nonce <= max_nonces && nonce <= u32::MAX as u64 {
                    if count % 512 == 0 && found.load(Ordering::Relaxed) {
                        break;
                    }
                    local[76..80].copy_from_slice(&(nonce as u32).to_le_bytes());
                    let mut h = vm.hash(&local);
                    count += 1;
                    h.reverse(); // LE -> BE
                    if &h <= target_be {
                        winner.store(nonce as u32, Ordering::SeqCst);
                        found.store(true, Ordering::SeqCst);
                        break;
                    }
                    nonce += threads as u64;
                }
                hashes.fetch_add(count, Ordering::Relaxed);
            });
        }
    });
    let total = hashes.load(Ordering::Relaxed);
    let stale = stale.load(Ordering::SeqCst);
    if !stale && found.load(Ordering::SeqCst) {
        (Some(winner.load(Ordering::SeqCst)), total, false)
    } else {
        (None, total, stale)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: mine-once <rpc_url> <user> <pass> <payout_addr> [max_blocks] [threads] [max_nonces]");
        eprintln!("       RPC credentials can also be given via BRISVIA_RPC_USER / BRISVIA_RPC_PASS (preferred; keeps them out of the command line)");
        std::process::exit(2);
    }
    let (url, addr) = (&args[1], &args[4]);
    // RPC credentials: prefer env vars (BRISVIA_RPC_USER/PASS) over argv. A process command line is readable by
    // other local processes, so the backend passes the node cookie user/password via env, not argv. Falls back to
    // argv[2]/argv[3] for direct CLI use.
    let user_s = std::env::var("BRISVIA_RPC_USER").ok().filter(|s| !s.is_empty())
        .unwrap_or_else(|| args.get(2).cloned().unwrap_or_default());
    let pass_s = std::env::var("BRISVIA_RPC_PASS").ok().filter(|s| !s.is_empty())
        .unwrap_or_else(|| args.get(3).cloned().unwrap_or_default());
    let (user, pass) = (user_s.as_str(), pass_s.as_str());
    let max_blocks: u64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(u64::MAX);
    let threads: usize = args.get(6).and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2));
    let max_nonces: u64 = args.get(7).and_then(|s| s.parse().ok()).unwrap_or(50_000_000);

    // JSON event mode (for Brisvia Desktop / sidecar): one JSON line per event to stdout.
    let json_mode = std::env::var("BRISVIA_JSON").is_ok();
    let ev = |obj: Value| {
        if json_mode { println!("{}", obj); }
    };

    // Pool mode (opt-in via env, set by the Tauri backend when the user picks a pool): mine against a stratum
    // pool instead of the local node. Only the public payout address (args[4]) travels, in the login. The solo
    // RPC path below is untouched — this branch returns before it.
    if let Ok(pool_url) = std::env::var("BRISVIA_POOL_URL") {
        let worker_name = std::env::var("BRISVIA_POOL_WORKER").unwrap_or_else(|_| "rig".to_string());
        // Encrypted by DEFAULT: if anything forgets to configure this, it fails on the safe side. The payout
        // address travels on this link, and on a plain one anyone in the middle can rewrite it and collect the
        // rewards. Only an explicit opt-out (the local e2e harness, which talks to loopback) turns it off.
        let tls = std::env::var("BRISVIA_POOL_PLAIN").is_err();
        let stop = AtomicBool::new(false);
        if json_mode { ev(json!({"event":"started","threads":threads,"mode":"pool"})); }
        brisvia_randomx::pool_worker::run_pool_worker(&pool_url, addr, &worker_name, threads, tls, &stop, |e| {
            use brisvia_randomx::pool_worker::PoolEvent;
            if !json_mode { return; }
            // Distinct pool events so the UI can be HONEST: a share SENT is not a share ACCEPTED, and a block is
            // not a share. The backend keeps separate counters and counts a contribution ONLY on ShareAccepted.
            // Disconnected goes out as JSON (not stderr) so the backend, which tails stdout, actually sees it.
            match e {
                PoolEvent::Connected => println!("{}", json!({"event":"pool_connected"})),
                PoolEvent::LoggedIn => println!("{}", json!({"event":"pool_login"})),
                PoolEvent::NewJob { .. } => println!("{}", json!({"event":"seed_ready"})),
                PoolEvent::ShareSubmitted { .. } => println!("{}", json!({"event":"share_submitted"})),
                PoolEvent::ShareAccepted => println!("{}", json!({"event":"share_accepted"})),
                PoolEvent::ShareRejected { reason } => println!("{}", json!({"event":"share_rejected","reason":reason})),
                PoolEvent::ShareStale => println!("{}", json!({"event":"stale"})),
                PoolEvent::BlockFound => println!("{}", json!({"event":"pool_block"})),
                PoolEvent::TargetChangeIgnored => {}
                PoolEvent::Disconnected(msg) => println!("{}", json!({"event":"pool_disconnected","reason":msg})),
            }
        });
        return;
    }

    let info = rpc(url, user, pass, "getaddressinfo", json!([addr])).unwrap();
    let payout_script = ScriptBuf::from_hex(info["scriptPubKey"].as_str().expect("scriptPubKey")).unwrap();

    if json_mode { ev(json!({"event":"started","threads":threads})); }
    else { println!("Brisvia miner · {threads} threads"); }
    let mut mined = 0u64;
    let mut cur_seed: Option<[u8; 32]> = None;
    let mut cache: Option<Arc<Cache>> = None;
    let mut dataset: Option<Arc<Dataset>> = None;
    let mut total_hashes = 0u64;
    let t_start = Instant::now();
    let mut consec_err = 0u32; // consecutive RPC errors -> circuit breaker, so a permanent failure stops instead of looping forever

    while mined < max_blocks {
        let tmpl = match rpc(url, user, pass, "getblocktemplate", json!([{"rules":["segwit"]}])) {
            Ok(t) => t,
            // Transient RPC hiccup: wait briefly and retry instead of dying (this used to stop the chain from advancing).
            Err(e) => {
                consec_err += 1;
                if consec_err >= 8 {
                    if json_mode { ev(json!({"event":"fatal","msg":format!("getblocktemplate: {e}")})); } else { eprintln!("stopping after repeated RPC errors: {e}"); }
                    break;
                }
                if json_mode { ev(json!({"event":"error","msg":format!("getblocktemplate: {e}")})); } else { eprintln!("getblocktemplate: {e}"); }
                std::thread::sleep(std::time::Duration::from_millis((consec_err as u64 * 700).min(5000))); // growing backoff
                continue;
            }
        };
        let height = tmpl["height"].as_u64().unwrap();
        let prev_hash = tmpl["previousblockhash"].as_str().unwrap().to_string();
        let (mut block, header_bytes, target_be, seed_key) = build_candidate(&tmpl, &payout_script, mined + 1);

        if cur_seed != Some(seed_key) {
            let t_ds = Instant::now();
            let c = Cache::new(&seed_key);
            let d = Dataset::new(&c, threads); // ~2.1GB, multithreaded init (once per seed)
            let ds_s = t_ds.elapsed().as_secs_f64();
            if json_mode { ev(json!({"event":"seed_ready","seconds":ds_s})); }
            else { eprintln!("(RandomX dataset ready in {ds_s:.0}s)"); }
            cache = Some(c);
            dataset = Some(d);
            cur_seed = Some(seed_key);
        }
        let cache_ref = cache.clone().unwrap();
        let dataset_ref = dataset.clone().unwrap();

        let t_block = Instant::now();
        let (found, hashes, stale) = mine_block(cache_ref, dataset_ref, &header_bytes, &target_be, threads, max_nonces, url, user, pass, &prev_hash, json_mode);
        total_hashes += hashes;

        if stale {
            if json_mode { ev(json!({"event":"stale","height":height})); }
            else { println!("Block {height}: a new block appeared on the network, retrying with a fresh template"); }
            continue;
        }
        match found {
            Some(nonce) => {
                block.header.nonce = nonce;
                let block_hex = hex::encode(serialize(&block));
                match rpc(url, user, pass, "submitblock", json!([block_hex])) {
                    Ok(v) if v.is_null() => {
                        consec_err = 0;
                        mined += 1;
                        let secs = t_block.elapsed().as_secs_f64();
                        let hs = hashes as f64 / secs.max(0.001);
                        if json_mode { ev(json!({"event":"accepted","height":height,"nonce":nonce,"seconds":secs,"hashrate":hs,"mined":mined})); }
                        else { println!("Block {height} ACCEPTED (nonce {nonce}) · {secs:.1}s · {hs:.0} H/s · mined={mined}"); }
                    }
                    Ok(v) => {
                        // Rejected (often a race: another block took this height). The RPC worked, so reset the counter and retry.
                        consec_err = 0;
                        if json_mode { ev(json!({"event":"rejected","height":height,"reason":v})); }
                        else { println!("Block {height} rejected: {v}"); }
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        continue;
                    }
                    Err(e) => {
                        consec_err += 1;
                        if consec_err >= 8 {
                            if json_mode { ev(json!({"event":"fatal","msg":e})); } else { println!("stopping after repeated submit errors: {e}"); }
                            break;
                        }
                        if json_mode { ev(json!({"event":"error","msg":e})); }
                        else { println!("submitblock error: {e}"); }
                        std::thread::sleep(std::time::Duration::from_millis((consec_err as u64 * 700).min(5000)));
                        continue;
                    }
                }
            }
            None => {
                // No nonce found in this batch: grab a fresh template and keep going instead of stopping.
                if json_mode { ev(json!({"event":"exhausted","height":height})); }
                else { println!("Block {height}: no nonce found in {max_nonces} attempts"); }
                continue;
            }
        }
    }
    let hs = total_hashes as f64 / t_start.elapsed().as_secs_f64().max(0.001);
    if json_mode { ev(json!({"event":"done","mined":mined,"total_hashes":total_hashes,"hashrate":hs})); }
    else { println!("--- done: {mined} blocks, {total_hashes} hashes, {hs:.0} H/s average ---"); }
}
