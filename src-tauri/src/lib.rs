// Backend of the Brisvia desktop app (Tauri). Rust controls the bitcoind node (sidecar), the credentials (RPC
// cookie), the wallet and mining; JavaScript (window.brisvia) only sees presentation data. Real node over local
// JSON-RPC, cookie auth, orderly shutdown (stop mining -> stop RPC -> kill).
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::str::FromStr;

use bip39::Mnemonic;
use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::secp256k1::Secp256k1;
use bitcoin::Network;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

const RPC_HOST: &str = "127.0.0.1";
const RPC_PORT: u16 = 19332; // local RPC of brisvia-test (the shared test network)
// Shared test network (testnet). The app used to run a private, isolated "regtest" chain (a demo); it now
// connects to brisvia-test, the network shared by all miners.
const NET_CHAIN: &str = "brisvia-test";      // network name passed to the node (-chain)
const NET_SUBDIR: &str = "brisvia-testnet";  // data subfolder the node creates for this network (cookie, wallets, blocks)
const WALLET_NAME: &str = "brisvia";

struct AppState {
    child: Arc<Mutex<Option<Child>>>,
    datadir: PathBuf,
    wallet_loaded: Arc<AtomicBool>,
    receive_addr: Arc<Mutex<String>>,
    mining: Arc<AtomicBool>,
    mined: Arc<AtomicU64>,
    mine_start: Arc<Mutex<Option<Instant>>>,
    intensity: Arc<Mutex<String>>,
    // The real mining engine process (sidecar brisvia-worker.exe, named differently from the window process so
    // they don't get confused in the task manager) and its last reported hashrate.
    miner_child: Arc<Mutex<Option<Child>>>,
    hashrate: Arc<Mutex<f64>>,
    // The engine takes a few seconds to build the RandomX dataset before the first contribution. Meanwhile the UI
    // shows "Preparing…" instead of a silent 0. Set to true on the engine's first event.
    miner_ready: Arc<AtomicBool>,
    tray_enabled: Arc<AtomicBool>,
    // Freshly generated mnemonic, kept ONLY in memory to verify the backup; wiped after confirmation. Never persisted.
    pending_mnemonic: Arc<Mutex<Option<String>>>,
    // Current language (es/en) and tray handle to rebuild its menu when the language changes.
    lang: Arc<Mutex<String>>,
    tray: Arc<Mutex<Option<tauri::tray::TrayIcon>>>,
    // Total CPU cores (fixed) and the miner's current thread count, to show processor usage.
    cores: usize,
    miner_threads: Arc<AtomicU64>,
    // Lifetime mining time in seconds, persisted to disk so the total survives restarts.
    total_mined_secs: Arc<Mutex<u64>>,
}

// ---- dependency-free base64 (for the cookie's Basic auth header) ----
fn b64(input: &[u8]) -> String {
    const C: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        out.push(C[(b0 >> 2) as usize] as char);
        out.push(C[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 { C[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { C[(b2 & 0x3f) as usize] as char } else { '=' });
    }
    out
}

fn read_cookie(datadir: &PathBuf) -> Option<String> {
    std::fs::read_to_string(datadir.join(NET_SUBDIR).join(".cookie")).ok()
}

// Maps common Core errors to a stable "ERR:CODE" that the UI translates; unknown ones pass through verbatim.
fn friendly_error(msg: &str) -> String {
    let m = msg.to_lowercase();
    if m.contains("insufficient funds") { return "ERR:INSUFFICIENT_FUNDS".into(); }
    if m.contains("invalid") && (m.contains("address") || m.contains("bech32")) { return "ERR:INVALID_ADDRESS".into(); }
    if m.contains("fee") && (m.contains("low") || m.contains("small") || m.contains("insufficient")) { return "ERR:FEE_TOO_LOW".into(); }
    if m.contains("starting") || m.contains("loading") || m.contains("warming up") || m.contains("rescanning") { return "ERR:NODE_STARTING".into(); }
    if m.contains("no available keys") || m.contains("not loaded") || m.contains("does not exist") { return "ERR:NODE_UNAVAILABLE".into(); }
    msg.to_string()
}

// ---- JSON-RPC call to bitcoind (cookie auth; the body is never logged) ----
fn rpc(datadir: &PathBuf, wallet: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
    let cookie = read_cookie(datadir).ok_or_else(|| "node is not ready yet".to_string())?;
    let url = match wallet {
        Some(w) => format!("http://{}:{}/wallet/{}", RPC_HOST, RPC_PORT, w),
        None => format!("http://{}:{}/", RPC_HOST, RPC_PORT),
    };
    let auth = format!("Basic {}", b64(cookie.trim().as_bytes()));
    let body = json!({ "jsonrpc": "1.0", "id": "brisvia", "method": method, "params": params });
    match ureq::post(&url)
        .set("Authorization", &auth)
        .timeout(Duration::from_secs(180))
        .send_json(body)
    {
        Ok(r) => {
            let v: Value = r.into_json().map_err(|e| e.to_string())?;
            if !v["error"].is_null() {
                return Err(friendly_error(v["error"]["message"].as_str().unwrap_or("node error")));
            }
            Ok(v["result"].clone())
        }
        Err(ureq::Error::Status(_, r)) => {
            let v: Value = r.into_json().unwrap_or_else(|_| json!({}));
            Err(friendly_error(v["error"]["message"].as_str().unwrap_or("node error")))
        }
        Err(e) => Err(e.to_string()),
    }
}

// Binary extension per platform: ".exe" on Windows, empty elsewhere (macOS/Linux).
#[cfg(windows)]
const EXE_SUFFIX: &str = ".exe";
#[cfg(not(windows))]
const EXE_SUFFIX: &str = "";

// ---- locate a bundled binary (prod: resource_dir; dev: manifest/binaries) ----
fn find_binary(app: &AppHandle, name: &str) -> Option<PathBuf> {
    if let Ok(res) = app.path().resource_dir() {
        for cand in [res.join("binaries").join(name), res.join(name)] {
            if cand.exists() {
                return Some(cand);
            }
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries").join(name);
    if dev.exists() {
        return Some(dev);
    }
    None
}

// On Windows, prevents child processes (node and miner) from opening a visible console window.
// It also unblocks the miner: a console-subsystem process launched from the GUI app (windows subsystem) with its
// output redirected to a pipe stays inert —never executing— unless this flag is passed. It's the same flag the
// Tauri sidecar plugin applies; here we use Command directly and must set it by hand. CREATE_NO_WINDOW = 0x08000000.
fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

// ---- start the node as a child process ----
fn start_node(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let bitcoind = find_binary(app, &format!("bitcoind{EXE_SUFFIX}")).ok_or("bitcoind not found")?;
    std::fs::create_dir_all(&state.datadir).map_err(|e| e.to_string())?;
    // Wide rpcthreads/rpcworkqueue: the miner (getblocktemplate + submitblock) plus the UI polling make several
    // concurrent RPC calls; with the defaults (4 threads) the node rejects connections under load.
    let conf = format!(
        // The node connects to the test network on its own (dnsseed + fixed seed), accepts inbound if the router
        // allows it (natpmp tries to open the port), and validates everything locally. RPC stays on 127.0.0.1 only.
        "chain={chain}\nserver=1\nlisten=1\ndiscover=1\ndnsseed=1\nnatpmp=1\nfallbackfee=0.0001\nrpcthreads=16\nrpcworkqueue=128\n[{chain}]\nrpcport={port}\nrpcbind=127.0.0.1\nrpcallowip=127.0.0.1\n",
        chain = NET_CHAIN,
        port = RPC_PORT
    );
    std::fs::write(state.datadir.join("bitcoin.conf"), conf).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&bitcoind);
    cmd.arg(format!("-datadir={}", state.datadir.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    no_window(&mut cmd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not start the node: {}", e))?;
    *state.child.lock().unwrap() = Some(child);
    Ok(())
}

// ---- orderly shutdown: stop mining -> stop RPC -> wait -> kill ----
fn stop_node(state: &AppState) {
    state.mining.store(false, Ordering::SeqCst);
    // stop the miner (sidecar) first so it stops asking the node for work
    if let Some(mut m) = state.miner_child.lock().unwrap().take() {
        let _ = m.kill();
        let _ = m.wait();
    }
    let _ = rpc(&state.datadir, None, "stop", json!([]));
    std::thread::sleep(Duration::from_millis(1500));
    if let Some(mut child) = state.child.lock().unwrap().take() {
        // give it a moment to close on its own; kill it otherwise
        for _ in 0..20 {
            match child.try_wait() {
                Ok(Some(_)) => return,
                _ => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        let _ = child.kill();
    }
}

// ================= commands exposed to the UI =================

#[tauri::command]
fn node_status(state: State<AppState>) -> Value {
    // Is there a "brisvia" wallet on disk? (so the UI waits for it to load instead of showing the onboarding)
    let on_disk = rpc(&state.datadir, None, "listwalletdir", json!([]))
        .map(|v| v["wallets"].as_array().map(|a| a.iter().any(|w| w["name"] == WALLET_NAME)).unwrap_or(false))
        .unwrap_or(false);
    match rpc(&state.datadir, None, "getblockchaininfo", json!([])) {
        Ok(info) => json!({
            "connected": true,
            "blocks": info["blocks"],
            "headers": info["headers"],
            "chain": info["chain"],
            // true = still syncing the shared chain (the UI disables "Mine" until it finishes).
            "ibd": info["initialblockdownload"],
            "verificationprogress": info["verificationprogress"],
            "walletReady": state.wallet_loaded.load(Ordering::SeqCst),
            "walletOnDisk": on_disk
        }),
        Err(_) => json!({ "connected": false, "walletReady": false, "walletOnDisk": false }),
    }
}

#[tauri::command]
fn wallet_exists(state: State<AppState>) -> bool {
    state.wallet_loaded.load(Ordering::SeqCst)
}

// The wallet is created during startup; create() just returns the receive address.
#[tauri::command]
fn wallet_create(state: State<AppState>) -> Result<Value, String> {
    let addr = state.receive_addr.lock().unwrap().clone();
    Ok(json!({ "address": addr }))
}

// Returns the 12 backup words: the in-memory mnemonic right after creation, or the persisted phrase later.
#[tauri::command]
fn wallet_seed(state: State<AppState>) -> Value {
    if let Some(p) = state.pending_mnemonic.lock().unwrap().clone() {
        return json!(p.split(' ').collect::<Vec<_>>());
    }
    json!(load_seed_phrase(&state.datadir))
}

#[tauri::command]
fn wallet_confirm_backup() -> Value {
    json!({ "backed_up": true })
}

#[tauri::command]
fn wallet_summary(state: State<AppState>) -> Value {
    let bal = rpc(&state.datadir, Some(WALLET_NAME), "getbalances", json!([])).unwrap_or_else(|_| json!({}));
    let trusted = bal["mine"]["trusted"].as_f64().unwrap_or(0.0);
    let pending = bal["mine"]["untrusted_pending"].as_f64().unwrap_or(0.0);
    let immature = bal["mine"]["immature"].as_f64().unwrap_or(0.0);
    json!({
        "balance": trusted,
        "immature": immature,
        "incoming": pending,
        "pending": pending + immature,
        "address": *state.receive_addr.lock().unwrap(),
        "backed_up": true
    })
}

#[tauri::command]
fn wallet_new_address(state: State<AppState>) -> Result<Value, String> {
    let addr = rpc(&state.datadir, Some(WALLET_NAME), "getnewaddress", json!([]))?;
    let a = addr.as_str().unwrap_or("").to_string();
    *state.receive_addr.lock().unwrap() = a.clone();
    Ok(json!({ "address": a }))
}

#[tauri::command]
fn wallet_send(state: State<AppState>, address: String, amount: f64) -> Result<Value, String> {
    if amount <= 0.0 {
        return Err("ERR:INVALID_AMOUNT".into());
    }
    let txid = rpc(&state.datadir, Some(WALLET_NAME), "sendtoaddress", json!([address, amount]))?;
    Ok(json!({ "ok": true, "txid": txid }))
}

#[tauri::command]
fn wallet_history(state: State<AppState>) -> Value {
    let txs = rpc(&state.datadir, Some(WALLET_NAME), "listtransactions", json!(["*", 25, 0]))
        .unwrap_or_else(|_| json!([]));
    let arr = txs.as_array().cloned().unwrap_or_default();
    // The UI localizes the label from `category`; this string is only a fallback.
    let out: Vec<Value> = arr
        .iter()
        .rev()
        .map(|t| {
            let amount = t["amount"].as_f64().unwrap_or(0.0);
            let cat = t["category"].as_str().unwrap_or("");
            let label = match cat {
                "generate" | "immature" => "Mined",
                "receive" => "Received",
                "send" => "Sent",
                _ => cat,
            };
            json!({
                "label": label,
                "amount": amount,
                "time": t["time"],
                "txid": t["txid"],
                "confirmations": t["confirmations"],
                "category": cat
            })
        })
        .collect();
    json!(out)
}

// ---- network info / technical data ----
#[tauri::command]
fn node_info(state: State<AppState>) -> Value {
    let chain = rpc(&state.datadir, None, "getblockchaininfo", json!([])).unwrap_or_else(|_| json!({}));
    let peers = rpc(&state.datadir, None, "getconnectioncount", json!([])).unwrap_or_else(|_| json!(0));
    let mining_info = rpc(&state.datadir, None, "getmininginfo", json!([])).unwrap_or_else(|_| json!({}));
    json!({
        "connected": !chain["blocks"].is_null(),
        "chain": chain["chain"],
        "blocks": chain["blocks"],
        "headers": chain["headers"],
        "difficulty": chain["difficulty"],
        "peers": peers,
        "verificationprogress": chain["verificationprogress"],
        // true while the node is still downloading and validating the shared chain (no mining during that time).
        "ibd": chain["initialblockdownload"],
        "bestblockhash": chain["bestblockhash"],
        "networkhashps": mining_info["networkhashps"]
    })
}

// ---- detail of a transaction (when a movement is clicked) ----
#[tauri::command]
fn tx_detail(state: State<AppState>, txid: String) -> Result<Value, String> {
    let tx = rpc(&state.datadir, Some(WALLET_NAME), "gettransaction", json!([txid]))?;
    Ok(json!({
        "txid": tx["txid"],
        "amount": tx["amount"],
        "confirmations": tx["confirmations"],
        "blockhash": tx["blockhash"],
        "blockheight": tx["blockheight"],
        "time": tx["time"],
        "fee": tx["fee"]
    }))
}

// ---- honest file backup (backupwallet). Does NOT fabricate a 12-word phrase. ----
#[tauri::command]
fn wallet_backup(state: State<AppState>) -> Result<Value, String> {
    let dir = docs_dir().join("Brisvia-backups");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("brisvia-wallet-{}.dat", ts));
    // bitcoind accepts / as separator on Windows
    let path_str = path.to_string_lossy().replace('\\', "/");
    rpc(&state.datadir, Some(WALLET_NAME), "backupwallet", json!([path_str]))?;
    Ok(json!({ "ok": true, "path": path.to_string_lossy() }))
}

// ---- current wallet type (for "Security and backup") ----
#[tauri::command]
fn wallet_kind(state: State<AppState>) -> Value {
    let info = rpc(&state.datadir, Some(WALLET_NAME), "getwalletinfo", json!([])).unwrap_or_else(|_| json!({}));
    json!({
        "name": info["walletname"],
        "descriptors": info["descriptors"],
        "kind": "brisvia_wallet",
        "has_seed_phrase": true
    })
}

// ================= BIP39: real 12-word backup =================
// Derives the (external, internal) descriptors WITHOUT checksum from a mnemonic. Testnet -> tprv, coin 1'.
// The descriptor carries the PRIVATE key (xprv) so the wallet can spend; Core derives and stores only that.
fn descriptors_from_mnemonic(mnemonic: &Mnemonic) -> Result<(String, String, String), String> {
    let seed = mnemonic.to_seed("");
    let secp = Secp256k1::new();
    let master = Xpriv::new_master(Network::Regtest, &seed).map_err(|e| e.to_string())?;
    let fp = master.fingerprint(&secp);
    let path = DerivationPath::from_str("84h/1h/0h").map_err(|e| e.to_string())?; // coin 1' = test
    let account = master.derive_priv(&secp, &path).map_err(|e| e.to_string())?;
    let origin = format!("[{}/84h/1h/0h]", fp);
    let external = format!("wpkh({}{}/0/*)", origin, account);
    let internal = format!("wpkh({}{}/1/*)", origin, account);
    Ok((fp.to_string(), external, internal))
}

// Adds ONLY the checksum to the PRIVATE descriptor (we don't use info["descriptor"], which returns the public/xpub
// version and would leave the wallet without spending keys). getdescriptorinfo["checksum"] is for the string we pass.
fn checksummed(datadir: &PathBuf, desc: &str) -> Result<String, String> {
    let info = rpc(datadir, None, "getdescriptorinfo", json!([desc]))?;
    let cs = info["checksum"].as_str().ok_or_else(|| "descriptor without checksum".to_string())?;
    Ok(format!("{}#{}", desc, cs))
}

// Imports the external+internal descriptors into a wallet. timestamp: "now" (new) or 0 (rescan on restore).
// Checks the result: if any import failed, returns the real error (never leave a wallet without keys).
fn import_descriptors(datadir: &PathBuf, wallet: &str, ext: &str, int: &str, rescan: bool) -> Result<(), String> {
    let ext_c = checksummed(datadir, ext)?;
    let int_c = checksummed(datadir, int)?;
    let ts: Value = if rescan { json!(0) } else { json!("now") };
    let res = rpc(datadir, Some(wallet), "importdescriptors", json!([[
        { "desc": ext_c, "active": true, "internal": false, "timestamp": ts, "range": [0, 1000] },
        { "desc": int_c, "active": true, "internal": true, "timestamp": ts, "range": [0, 1000] }
    ]]))?;
    if let Some(arr) = res.as_array() {
        for item in arr {
            if !item["success"].as_bool().unwrap_or(false) {
                let err = item["error"]["message"].as_str().unwrap_or("could not import descriptor");
                return Err(format!("importdescriptors: {}", err));
            }
        }
    }
    Ok(())
}

// If it's the main wallet, mark it active (receive address + wallet_loaded) so the app can operate.
fn activate_wallet(state: &AppState, name: &str) {
    if name == WALLET_NAME {
        if let Ok(addr) = rpc(&state.datadir, Some(name), "getnewaddress", json!([])) {
            if let Some(a) = addr.as_str() {
                *state.receive_addr.lock().unwrap() = a.to_string();
            }
        }
        state.wallet_loaded.store(true, Ordering::SeqCst);
    }
}

// Create a NEW wallet with a 12-word backup. Returns the words (to show once) + fingerprint.
#[tauri::command]
fn wallet_create_bip39(state: State<AppState>, name: String) -> Result<Value, String> {
    let mnemonic = Mnemonic::generate(12).map_err(|e| e.to_string())?;
    let (fp, ext, int) = descriptors_from_mnemonic(&mnemonic)?;
    // createwallet blank (no keys), descriptor
    rpc(&state.datadir, None, "createwallet", json!([name, false, true, "", false, true]))?;
    import_descriptors(&state.datadir, &name, &ext, &int, false)?;
    let words = mnemonic.to_string();
    *state.pending_mnemonic.lock().unwrap() = Some(words.clone());
    save_seed_phrase(&state.datadir, &words);
    activate_wallet(&state, &name);
    let list: Vec<&str> = words.split(' ').collect();
    Ok(json!({ "words": list, "fingerprint": fp }))
}

// Verify the backup: compare, in order, the words the user enters against the mnemonic in memory.
// If they match, the mnemonic is wiped from memory (no longer needed).
#[tauri::command]
fn wallet_verify_backup(state: State<AppState>, words: Vec<String>) -> Value {
    let pending = state.pending_mnemonic.lock().unwrap().clone();
    let ok = pending
        .map(|p| p.split(' ').map(|s| s.to_string()).collect::<Vec<_>>() == words)
        .unwrap_or(false);
    if ok {
        *state.pending_mnemonic.lock().unwrap() = None;
    }
    json!({ "ok": ok })
}

// Restore from 12 words into a NEW wallet (never overwrites an existing one). Rescan from genesis.
#[tauri::command]
fn wallet_restore_bip39(state: State<AppState>, phrase: String, name: String) -> Result<Value, String> {
    let mnemonic = Mnemonic::parse(phrase.trim())
        .map_err(|_| "ERR:INVALID_PHRASE".to_string())?;
    let (fp, ext, int) = descriptors_from_mnemonic(&mnemonic)?;
    // new name: NEVER overwrites a wallet with funds
    rpc(&state.datadir, None, "createwallet", json!([name, false, true, "", false, true]))?;
    import_descriptors(&state.datadir, &name, &ext, &int, true)?; // rescan from genesis
    activate_wallet(&state, &name);
    save_seed_phrase(&state.datadir, phrase.trim());
    Ok(json!({ "ok": true, "fingerprint": fp }))
}

// ---- mining: launch the engine sidecar and follow its accepted-block events ----
// Lifetime mining time is persisted in a small text file in the data directory.
fn total_mined_path(datadir: &std::path::Path) -> std::path::PathBuf { datadir.join("mined_total_secs.txt") }
fn load_total_mined(datadir: &std::path::Path) -> u64 {
    std::fs::read_to_string(total_mined_path(datadir)).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}
fn save_total_mined(datadir: &std::path::Path, secs: u64) {
    let _ = std::fs::create_dir_all(datadir);
    let _ = std::fs::write(total_mined_path(datadir), secs.to_string());
}

// The 12-word backup phrase is persisted locally so the user can view or verify it later (Security screen).
// It sits next to the wallet, which is already unencrypted on disk; for mainnet this should be encrypted.
fn seed_phrase_path(datadir: &std::path::Path) -> std::path::PathBuf { datadir.join("wallet_seed_phrase.txt") }
fn save_seed_phrase(datadir: &std::path::Path, phrase: &str) {
    let _ = std::fs::create_dir_all(datadir);
    let _ = std::fs::write(seed_phrase_path(datadir), phrase.trim());
}
fn load_seed_phrase(datadir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(seed_phrase_path(datadir)).ok()
        .map(|s| s.trim().split_whitespace().map(|w| w.to_string()).collect())
        .unwrap_or_default()
}

#[tauri::command]
fn miner_start(app: AppHandle, state: State<AppState>, intensity: Option<String>) -> Value {
    if let Some(i) = intensity {
        *state.intensity.lock().unwrap() = i;
    }
    if state.mining.swap(true, Ordering::SeqCst) {
        return json!({ "mining": true }); // already mining
    }
    *state.mine_start.lock().unwrap() = Some(Instant::now());
    *state.hashrate.lock().unwrap() = 0.0;
    state.miner_ready.store(false, Ordering::SeqCst); // starts "preparing" until the engine's first event

    // How many cores, based on the chosen intensity.
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let threads = match state.intensity.lock().unwrap().as_str() {
        "suave" => 1usize,
        "intenso" => cores,
        _ => (cores / 2).max(1),
    };
    state.miner_threads.store(threads as u64, Ordering::SeqCst);

    // Node cookie auth (never exposed in logs): the .cookie file contains "user:password".
    let cookie = std::fs::read_to_string(state.datadir.join(NET_SUBDIR).join(".cookie")).unwrap_or_default();
    let (cuser, cpass) = cookie.split_once(':').map(|(u, p)| (u.to_string(), p.to_string()))
        .unwrap_or_else(|| ("__cookie__".into(), String::new()));
    // Address that receives the rewards. If startup hasn't loaded one yet (wallet just created, or the node was slow
    // to come up), request a fresh one from the wallet. Never launch the miner with an empty address: it would
    // produce invalid blocks and no contribution, wasting the dataset preparation.
    let mut addr = state.receive_addr.lock().unwrap().clone();
    if addr.trim().is_empty() {
        if let Ok(a) = rpc(&state.datadir, Some(WALLET_NAME), "getnewaddress", json!([])) {
            if let Some(s) = a.as_str() {
                addr = s.to_string();
                *state.receive_addr.lock().unwrap() = addr.clone();
            }
        }
    }
    if addr.trim().is_empty() {
        state.mining.store(false, Ordering::SeqCst);
        return json!({ "mining": false, "error": "ERR:WALLET_NOT_READY" });
    }
    let url = format!("http://127.0.0.1:{}/wallet/{}", RPC_PORT, WALLET_NAME);

    let miner_bin = match find_binary(&app, &format!("brisvia-worker{EXE_SUFFIX}")) {
        Some(b) => b,
        None => { state.mining.store(false, Ordering::SeqCst); return json!({ "mining": false, "error": "ERR:MINER_NOT_FOUND" }); }
    };

    // Miner events: we do NOT read its stdout over a pipe. On Windows, a child process launched from the GUI app
    // with piped output BLOCKS on its first event write if the reader doesn't drain in time (this was the cause of
    // mining not starting). Instead we redirect its stdout to a file —which never blocks on write, like the node—
    // and follow it (tail) from a thread that re-reads the file every half second.
    let events_path = state.datadir.join("miner-events.log");
    let out_stdio = std::fs::File::create(&events_path).map(Stdio::from).unwrap_or_else(|_| Stdio::null());
    let mut cmd = Command::new(&miner_bin);
    cmd.args([&url, &cuser, &cpass, &addr, &u64::MAX.to_string(), &threads.to_string()])
        .env("BRISVIA_JSON", "1")
        .stdout(out_stdio)
        .stderr(Stdio::null());
    no_window(&mut cmd);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => { state.mining.store(false, Ordering::SeqCst); return json!({ "mining": false, "error": format!("could not start the miner: {e}") }); }
    };

    // Thread that follows the events file and updates accepted contributions + real hashrate.
    {
        let mined = state.mined.clone();
        let hashrate = state.hashrate.clone();
        let mining = state.mining.clone();
        let ready = state.miner_ready.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let mut prev_total: u64 = 0;
            loop {
                if !mining.load(Ordering::SeqCst) { break; }
                std::thread::sleep(Duration::from_millis(500));
                let f = match std::fs::File::open(&events_path) { Ok(f) => f, Err(_) => continue };
                let mut total = 0u64;
                let mut last_hs: Option<f64> = None;
                for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
                    if let Ok(evt) = serde_json::from_str::<Value>(&line) {
                        match evt["event"].as_str() {
                            // Dataset ready or first block: the engine is working, stop showing "Preparing…".
                            Some("seed_ready") => ready.store(true, Ordering::SeqCst),
                            Some("accepted") => {
                                ready.store(true, Ordering::SeqCst);
                                total += 1;
                                if let Some(hs) = evt["hashrate"].as_f64() { last_hs = Some(hs); }
                            }
                            _ => {}
                        }
                    }
                }
                if total > prev_total {
                    mined.fetch_add(total - prev_total, Ordering::SeqCst);
                    prev_total = total;
                    if let Some(hs) = last_hs { *hashrate.lock().unwrap() = hs; }
                }
            }
        });
    }
    *state.miner_child.lock().unwrap() = Some(child);
    json!({ "mining": true })
}

#[tauri::command]
fn miner_stop(state: State<AppState>) -> Value {
    state.mining.store(false, Ordering::SeqCst);
    // Add this session's time to the lifetime total and persist it.
    let sess = state.mine_start.lock().unwrap().map(|t| t.elapsed().as_secs()).unwrap_or(0);
    if sess > 0 {
        let mut tot = state.total_mined_secs.lock().unwrap();
        *tot += sess;
        save_total_mined(&state.datadir, *tot);
    }
    *state.mine_start.lock().unwrap() = None;
    state.miner_threads.store(0, Ordering::SeqCst);
    if let Some(mut child) = state.miner_child.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    *state.hashrate.lock().unwrap() = 0.0;
    state.miner_ready.store(false, Ordering::SeqCst);
    json!({ "mining": false })
}

#[tauri::command]
fn miner_set_intensity(state: State<AppState>, intensity: String) -> Value {
    *state.intensity.lock().unwrap() = intensity.clone();
    json!({ "intensity": intensity })
}

#[tauri::command]
fn miner_status(state: State<AppState>) -> Value {
    let mining = state.mining.load(Ordering::SeqCst);
    let secs = state
        .mine_start
        .lock()
        .unwrap()
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);
    let intensity = state.intensity.lock().unwrap().clone();
    // "preparing" = mining but the engine is still building the RandomX dataset (no event received yet).
    let preparing = mining && !state.miner_ready.load(Ordering::SeqCst);
    let total = *state.total_mined_secs.lock().unwrap() + if mining { secs } else { 0 };
    json!({
        "mining": mining,
        "preparing": preparing,
        "accepted": state.mined.load(Ordering::SeqCst),
        "secondsMining": if mining { secs } else { 0 },
        // REAL hashrate reported by the miner (CPU hashes per second)
        "hashrate": if mining { *state.hashrate.lock().unwrap() } else { 0.0 },
        "intensity": intensity,
        "threads": state.miner_threads.load(Ordering::SeqCst),
        "cores": state.cores as u64,
        "totalSeconds": total
    })
}

// ================= settings =================
#[tauri::command]
fn settings_get(app: AppHandle, state: State<AppState>) -> Value {
    use tauri_plugin_autostart::ManagerExt;
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    json!({
        "autostart": autostart,
        "tray": state.tray_enabled.load(Ordering::SeqCst),
        "defaultIntensity": *state.intensity.lock().unwrap()
    })
}

#[tauri::command]
fn settings_set(app: AppHandle, state: State<AppState>, key: String, value: Value) -> Value {
    use tauri_plugin_autostart::ManagerExt;
    match key.as_str() {
        "tray" => state.tray_enabled.store(value.as_bool().unwrap_or(true), Ordering::SeqCst),
        "defaultIntensity" => {
            if let Some(s) = value.as_str() { *state.intensity.lock().unwrap() = s.to_string(); }
        }
        "autostart" => {
            let al = app.autolaunch();
            let _ = if value.as_bool().unwrap_or(false) { al.enable() } else { al.disable() };
        }
        _ => {}
    }
    json!({ "ok": true })
}

// Open a link (web/social) in the system browser, not inside the webview.
#[tauri::command]
fn open_url(url: String) {
    // http/https only, for safety
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("cmd").args(["/C", "start", "", &url]).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(&url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = Command::new("xdg-open").arg(&url).spawn();
    }
}

// ---- system locale (to pick es/en the first time) ----
#[tauri::command]
fn system_locale() -> String {
    sys_locale::get_locale().unwrap_or_else(|| "en".to_string())
}

// Tray menu labels by language.
fn tray_labels(lang: &str) -> (&'static str, &'static str) {
    if lang == "es" { ("Abrir Brisvia", "Salir de Brisvia") } else { ("Open Brisvia", "Exit Brisvia") }
}

fn build_tray_menu(app: &AppHandle, lang: &str) -> Result<tauri::menu::Menu<tauri::Wry>, tauri::Error> {
    use tauri::menu::{MenuBuilder, MenuItem};
    let (open, quit) = tray_labels(lang);
    let show_i = MenuItem::with_id(app, "show", open, true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", quit, true, None::<&str>)?;
    MenuBuilder::new(app).item(&show_i).item(&quit_i).build()
}

// Change language: store it and rebuild the translated tray menu (the UI translates itself).
#[tauri::command]
fn set_language(app: AppHandle, state: State<AppState>, lang: String) {
    *state.lang.lock().unwrap() = lang.clone();
    if let Some(tray) = state.tray.lock().unwrap().as_ref() {
        if let Ok(menu) = build_tray_menu(&app, &lang) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

// ---- Auto-updater --------------------------------------------------------
// Lets an installed copy update itself, so users who already downloaded the miner
// don't have to re-download it when real mining starts. Updates are signed (minisign);
// the download step verifies the signature before anything is installed.

// Builds an Updater. Honors BRISVIA_UPDATE_ENDPOINT (end-to-end testing only); otherwise
// uses the endpoint from tauri.conf (GitHub Releases).
fn build_updater(app: &tauri::AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    use tauri_plugin_updater::UpdaterExt;
    match std::env::var("BRISVIA_UPDATE_ENDPOINT") {
        Ok(ep) if !ep.is_empty() => {
            let url = tauri::Url::parse(&ep).map_err(|e| e.to_string())?;
            app.updater_builder()
                .endpoints(vec![url])
                .map_err(|e| e.to_string())?
                .build()
                .map_err(|e| e.to_string())
        }
        _ => app.updater().map_err(|e| e.to_string()),
    }
}

async fn check_update_inner(app: &tauri::AppHandle) -> Result<serde_json::Value, String> {
    let updater = build_updater(app)?;
    match updater.check().await {
        Ok(Some(u)) => Ok(json!({
            "available": true,
            "version": u.version,
            "currentVersion": u.current_version,
            "notes": u.body,
        })),
        Ok(None) => Ok(json!({
            "available": false,
            "currentVersion": app.package_info().version.to_string(),
        })),
        Err(e) => Err(e.to_string()),
    }
}

// Frontend: is there a newer signed version available?
#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    check_update_inner(&app).await
}

// Frontend: download the newer version (verifies signature), install it and relaunch.
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    let updater = build_updater(&app)?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no update available".to_string())?;
    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // data directory next to the user's app data
    let datadir = dirs_data_dir().join("BrisviaSim");
    let total_secs_initial = load_total_mined(&datadir);
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);

    let state = AppState {
        child: Arc::new(Mutex::new(None)),
        datadir,
        wallet_loaded: Arc::new(AtomicBool::new(false)),
        receive_addr: Arc::new(Mutex::new(String::new())),
        mining: Arc::new(AtomicBool::new(false)),
        mined: Arc::new(AtomicU64::new(0)),
        mine_start: Arc::new(Mutex::new(None)),
        intensity: Arc::new(Mutex::new("equilibrado".to_string())),
        miner_child: Arc::new(Mutex::new(None)),
        hashrate: Arc::new(Mutex::new(0.0)),
        miner_ready: Arc::new(AtomicBool::new(false)),
        tray_enabled: Arc::new(AtomicBool::new(true)),
        pending_mnemonic: Arc::new(Mutex::new(None)),
        lang: Arc::new(Mutex::new("es".to_string())),
        tray: Arc::new(Mutex::new(None)),
        cores,
        miner_threads: Arc::new(AtomicU64::new(0)),
        total_mined_secs: Arc::new(Mutex::new(total_secs_initial)),
    };

    tauri::Builder::default()
        // Single instance: if the app is already open and it's launched again, instead of opening another window
        // (and another mining engine), bring the existing window to the front. Must be the first plugin registered.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .setup(|app| {
            // Headless end-to-end self-test of the updater: check the endpoint and
            // download + verify the signature WITHOUT installing. Enabled via env var, used to
            // verify the updater before shipping. Skips node/tray/wallet startup.
            if std::env::var("BRISVIA_UPDATE_SELFTEST").is_ok() {
                let h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut out = String::new();
                    match build_updater(&h) {
                        Ok(updater) => match updater.check().await {
                            Ok(Some(update)) => {
                                out.push_str(&format!("SELFTEST_CHECK available version={} current={}\n", update.version, update.current_version));
                                match update.download(|_c, _t| {}, || {}).await {
                                    Ok(bytes) => out.push_str(&format!("SELFTEST_DOWNLOAD ok signature_valid bytes={}\n", bytes.len())),
                                    Err(e) => out.push_str(&format!("SELFTEST_DOWNLOAD fail {}\n", e)),
                                }
                            }
                            Ok(None) => out.push_str("SELFTEST_CHECK none already_latest\n"),
                            Err(e) => out.push_str(&format!("SELFTEST_CHECK error {}\n", e)),
                        },
                        Err(e) => out.push_str(&format!("SELFTEST_UPDATER error {}\n", e)),
                    }
                    print!("{}", out);
                    if let Ok(p) = std::env::var("BRISVIA_SELFTEST_OUT") { let _ = std::fs::write(&p, &out); }
                    h.exit(0);
                });
                return Ok(());
            }
            let handle = app.handle().clone();
            let state = app.state::<AppState>();
            // start the node
            if let Err(e) = start_node(&handle, &state) {
                eprintln!("[brisvia] could not start the node: {}", e);
            }
            // System tray: notification-area icon + menu (Open / Exit).
            {
                use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
                let menu = build_tray_menu(&handle, "es")?;
                let mut builder = TrayIconBuilder::new()
                    .tooltip("Brisvia")
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "quit" => {
                            let state = app.state::<AppState>();
                            stop_node(&state);
                            app.exit(0);
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                            let app = tray.app_handle();
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    });
                if let Some(icon) = app.default_window_icon() {
                    builder = builder.icon(icon.clone());
                }
                let tray_icon = builder.build(app)?;
                *state.tray.lock().unwrap() = Some(tray_icon);
            }
            // thread: wait for RPC to be ready and load/create the wallet + first address
            let datadir = state.datadir.clone();
            let wallet_loaded = state.wallet_loaded.clone();
            let receive_addr = state.receive_addr.clone();
            std::thread::spawn(move || {
                for _ in 0..90 {
                    if rpc(&datadir, None, "getblockcount", json!([])).is_ok() {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                // Load the "brisvia" wallet ONLY if it already exists (created earlier with 12 words). If it does
                // not exist, it is NOT created automatically: the UI shows the onboarding to create or import.
                let exists = rpc(&datadir, None, "listwalletdir", json!([]))
                    .map(|v| {
                        v["wallets"].as_array().map(|a| a.iter().any(|w| w["name"] == WALLET_NAME)).unwrap_or(false)
                    })
                    .unwrap_or(false);
                if exists {
                    let _ = rpc(&datadir, None, "loadwallet", json!([WALLET_NAME]));
                    if let Ok(addr) = rpc(&datadir, Some(WALLET_NAME), "getnewaddress", json!([])) {
                        if let Some(a) = addr.as_str() {
                            *receive_addr.lock().unwrap() = a.to_string();
                        }
                    }
                    wallet_loaded.store(true, Ordering::SeqCst);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let state = app.state::<AppState>();
                if state.tray_enabled.load(Ordering::SeqCst) {
                    // Tray enabled: closing hides the window (keeps mining). "Exit" from the tray really closes.
                    let _ = window.hide();
                    api.prevent_close();
                }
                // Tray disabled: allow the close; RunEvent::ExitRequested stops the node.
            }
        })
        .invoke_handler(tauri::generate_handler![
            node_status,
            wallet_exists,
            wallet_create,
            wallet_seed,
            wallet_confirm_backup,
            wallet_summary,
            wallet_new_address,
            wallet_send,
            wallet_history,
            miner_start,
            miner_stop,
            miner_set_intensity,
            miner_status,
            node_info,
            tx_detail,
            wallet_backup,
            wallet_kind,
            wallet_create_bip39,
            wallet_verify_backup,
            wallet_restore_bip39,
            settings_get,
            settings_set,
            open_url,
            system_locale,
            set_language,
            check_update,
            install_update
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Brisvia")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state = app_handle.state::<AppState>();
                stop_node(&state);
            }
        });
}

// User's Documents folder (for wallet backups).
fn docs_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(up) = std::env::var("USERPROFILE") {
            return PathBuf::from(up).join("Documents");
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join("Documents");
        }
    }
    dirs_data_dir()
}

// User data directory, cross-platform with no extra dependencies.
fn dirs_data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("Brisvia");
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".brisvia");
        }
    }
    std::env::temp_dir().join("Brisvia")
}
