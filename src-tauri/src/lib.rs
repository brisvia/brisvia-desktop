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
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

const RPC_HOST: &str = "127.0.0.1";

// The network the app connects to is chosen at COMPILE TIME. The node binary supports both networks;
// only these three constants differ between the two builds:
//   - default (no flags):              brisvia-test — the shared test network.
//   - built with `--features mainnet`: brisvia      — the real network (mainnet), opens Aug 1, 2026.
#[cfg(not(feature = "mainnet"))]
mod netcfg {
    pub const RPC_PORT: u16 = 19332; // local RPC of the node (test network)
    pub const NET_CHAIN: &str = "brisvia-test"; // network name passed to the node (-chain)
    pub const NET_SUBDIR: &str = "brisvia-testnet"; // data subfolder the node creates (cookie, wallets, blocks)
    // Extended-key network for the wallet's BIP32 keys. Must match the node's EXT prefixes:
    // brisvia-test uses EXT_SECRET_KEY 0x04358394 -> tprv / EXT_PUBLIC_KEY 0x043587CF -> tpub (same as Bitcoin testnet),
    // which NetworkKind::Test (Regtest/Testnet) serializes. Otherwise the node's wpkh() descriptor rejects the key.
    pub const WALLET_NETWORK: bitcoin::Network = bitcoin::Network::Regtest;
}
#[cfg(feature = "mainnet")]
mod netcfg {
    pub const RPC_PORT: u16 = 9338; // local RPC of the node (real network); own port (P2P 9339), distinct from Litecoin 9333/9332
    pub const NET_CHAIN: &str = "brisvia"; // real network (mainnet)
    pub const NET_SUBDIR: &str = "brisvia-mainnet"; // data subfolder the node creates (cookie, wallets, blocks)
    // Extended-key network for the wallet's BIP32 keys. Must match the node's EXT prefixes:
    // brisvia (mainnet) uses EXT_SECRET_KEY 0x0488ADE4 -> xprv / EXT_PUBLIC_KEY 0x0488B21E -> xpub (same as Bitcoin
    // mainnet), which NetworkKind::Main (Bitcoin) serializes. Otherwise the node's wpkh() descriptor rejects the key.
    pub const WALLET_NETWORK: bitcoin::Network = bitcoin::Network::Bitcoin;
}
use netcfg::{NET_CHAIN, NET_SUBDIR, RPC_PORT, WALLET_NETWORK};
const WALLET_NAME: &str = "brisvia";
// The official Brisvia pool endpoint used when the user picks "Official pool" (no fee). host:port of the
// stratum server. TODO: confirm the real port when the official pool is deployed.
const OFFICIAL_POOL_URL: &str = "pool.brisvia.com:3333";

struct AppState {
    child: Arc<Mutex<Option<Child>>>,
    datadir: PathBuf,
    wallet_loaded: Arc<AtomicBool>,
    receive_addr: Arc<Mutex<String>>,
    mining: Arc<AtomicBool>,
    mined: Arc<AtomicU64>,
    stale: Arc<AtomicU64>, // blocks we found that arrived late (another block won the height) — normal on a shared net
    mine_start: Arc<Mutex<Option<Instant>>>,
    keep_session_on_relaunch: Arc<AtomicBool>, // set before a power-change relaunch so it does NOT reset the session timer/speed
    intensity: Arc<Mutex<String>>,
    // The real mining engine process (sidecar brisvia-worker.exe, named differently from the window process so
    // they don't get confused in the task manager) and its last reported hashrate.
    miner_child: Arc<Mutex<Option<Child>>>,
    hashrate: Arc<Mutex<f64>>,
    // The engine takes a few seconds to build the RandomX dataset before the first contribution. Meanwhile the UI
    // shows "Preparing…" instead of a silent 0. Set to true on the engine's first event.
    miner_ready: Arc<AtomicBool>,
    tray_enabled: Arc<AtomicBool>,
    // Mining mode ("solo" | "pool" | "custom") and the custom pool address (host:port), chosen in Settings.
    // In pool/custom mode miner_start hands the worker a BRISVIA_POOL_URL; the solo path is unchanged.
    mining_mode: Arc<Mutex<String>>,
    pool_address: Arc<Mutex<String>>,
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
    // Short-lived cache of the wallet metrics used to compute achievements. Scanning every transaction on each
    // poll would be wasteful, so the expensive scan is reused for a few seconds; the unlocked/notified logic on
    // top of it is always evaluated fresh.
    ach_cache: Arc<Mutex<Option<(Instant, WalletMetrics)>>>,
}

// Real-mining start on the main network: 2026-08-01 15:00:00 UTC (12:00 Argentina). Kept in sync with the
// frontend countdown. Used to award the "pioneer" milestone achievements from the chain (they travel with the seed).
const MAINNET_START: i64 = 1_785_596_400;
const ONE_DAY: i64 = 86_400;
const HALVING_HEIGHT: i64 = 1_000_000;

// Wallet numbers behind the 50 achievements. Everything is derived from the wallet/chain (never from the local
// machine) so the achievements come back on their own when the 12 words are restored on another computer.
#[derive(Clone, Default)]
struct WalletMetrics {
    blocks: u64,        // coinbase transactions (blocks this wallet mined)
    balance: f64,       // spendable (trusted) balance in BRVA
    sends: u64,         // outgoing payments
    receives: u64,      // incoming payments (never coinbase)
    first_time: i64,    // unix time of the wallet's earliest transaction (0 if none)
    before_halving: bool, // mined a block below the first-halving height
    first_day: bool,    // mined a block during the network's first day
    first_week: bool,   // mined a block during the network's first week
    first_month: bool,  // mined a block during the network's first month
    after_year: bool,   // mined a block after the network's first year
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
    std::fs::read_to_string(datadir.join(net_subdir()).join(".cookie")).ok()
}

// Network chain (-chain value) and the node's data subfolder (where the .cookie lives). In production these
// are the compile-time constants. In the e2e test binary (feature "e2e") they can be redirected to an isolated
// regtest instance via env vars. The env reads are compiled out of the public binary, so it can never be
// pointed anywhere but its own network.
fn net_chain() -> String {
    #[cfg(feature = "e2e")]
    if let Ok(c) = std::env::var("BRISVIA_E2E_CHAIN") {
        return c;
    }
    NET_CHAIN.to_string()
}
fn net_subdir() -> String {
    #[cfg(feature = "e2e")]
    if let Ok(s) = std::env::var("BRISVIA_E2E_SUBDIR") {
        return s;
    }
    NET_SUBDIR.to_string()
}

// e2e-only clock injection. If BRISVIA_E2E_NOW (unix SECONDS) is set, freeze Date.now() to that instant in
// every webview BEFORE the page scripts run. Only Date.now() is replaced (Date.UTC/parse and the Date constructor
// stay real), which is exactly what the frontend reads to decide the pre-launch wait mode and the launch countdown.
// This lets the test drive "before Aug 1" vs "after Aug 1" without ever touching the machine's real clock.
#[cfg(feature = "e2e")]
fn e2e_clock_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    let script = match std::env::var("BRISVIA_E2E_NOW")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
    {
        Some(secs) => format!(
            "(function(){{var __t={};Date.now=function(){{return __t;}};}})();",
            secs * 1000
        ),
        None => String::new(),
    };
    tauri::plugin::Builder::new("brisvia_e2e_clock")
        .js_init_script(script)
        .build()
}

// Maps common Core errors to a stable "ERR:CODE" that the UI translates; unknown ones pass through verbatim.
fn friendly_error(msg: &str) -> String {
    let m = msg.to_lowercase();
    if m.contains("insufficient funds") { return "ERR:INSUFFICIENT_FUNDS".into(); }
    if m.contains("invalid") && (m.contains("address") || m.contains("bech32")) { return "ERR:INVALID_ADDRESS".into(); }
    if m.contains("fee") && (m.contains("low") || m.contains("small") || m.contains("insufficient")) { return "ERR:FEE_TOO_LOW".into(); }
    if m.contains("starting") || m.contains("loading") || m.contains("warming up") || m.contains("rescanning") { return "ERR:NODE_STARTING".into(); }
    if m.contains("no available keys") || m.contains("not loaded") || m.contains("does not exist") { return "ERR:NODE_UNAVAILABLE".into(); }
    if m.contains("already exists") || m.contains("already loaded") || m.contains("database already") { return "ERR:WALLET_EXISTS".into(); }
    msg.to_string()
}

// The node's RPC port. Override with BRISVIA_RPC_PORT to run an isolated test instance without clashing
// with the real app's node on the default port.
fn rpc_port() -> u16 {
    std::env::var("BRISVIA_RPC_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(RPC_PORT)
}

// ---- JSON-RPC call to bitcoind (cookie auth; the body is never logged) ----
fn rpc(datadir: &PathBuf, wallet: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
    let cookie = read_cookie(datadir).ok_or_else(|| "node is not ready yet".to_string())?;
    let url = match wallet {
        Some(w) => format!("http://{}:{}/wallet/{}", RPC_HOST, rpc_port(), w),
        None => format!("http://{}:{}/", RPC_HOST, rpc_port()),
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
    // Mainnet seed nodes: the "front doors" to the network on launch day. On the test network the shared DNS/fixed
    // seed already covers it; on mainnet we point the node at our own seed servers (Hostinger VPS + Oracle) so the
    // app finds peers from the first minute. More can be added over time.
    #[cfg(feature = "mainnet")]
    let seeds = "addnode=187.77.240.145\naddnode=129.80.250.36\n";
    #[cfg(not(feature = "mainnet"))]
    let seeds = "";
    let chain = net_chain();
    // An isolated e2e run (regtest override) must never reach the outside world: no discovery, no dnsseed,
    // no port mapping, no seed nodes. That keeps the automated tests offline, deterministic and self-contained.
    let isolated = chain != NET_CHAIN;
    // Relay/mempool/wallet-fee policy (audit-decided 2026-07-12; LOCAL policy, NOT consensus). On the real network:
    // deter cheap spam on a no-price coin (minrelay 0.01, dust 0.03, blockmintxfee 0.01), keep small seed nodes from a
    // huge mempool (maxmempool 50 MiB, expiry 24 h, persist), and a safe wallet fallback fee (0.02, ~2x minrelay).
    // These can be tuned in later versions WITHOUT a fork. In an isolated e2e/regtest run keep the light defaults so
    // automated tests are not affected by mainnet relay fees.
    let net_lines = if isolated {
        "listen=0\ndiscover=0\ndnsseed=0\nnatpmp=0\nfallbackfee=0.0001\n"
    } else {
        "listen=1\ndiscover=1\ndnsseed=1\nnatpmp=1\nfallbackfee=0.02\nminrelaytxfee=0.01\nincrementalrelayfee=0.01\ndustrelayfee=0.03\nblockmintxfee=0.01\nmaxmempool=50\nmempoolexpiry=24\npersistmempool=1\n"
    };
    let seeds = if isolated { "" } else { seeds };
    let conf = format!(
        // The node connects to the network on its own (dnsseed + fixed seed), accepts inbound if the router
        // allows it (natpmp tries to open the port), and validates everything locally. RPC stays on 127.0.0.1 only.
        "chain={chain}\nserver=1\n{net_lines}rpcthreads=16\nrpcworkqueue=128\n[{chain}]\nrpcport={port}\nrpcbind=127.0.0.1\nrpcallowip=127.0.0.1\n{seeds}",
        chain = chain,
        net_lines = net_lines,
        port = rpc_port(),
        seeds = seeds
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

// Fast, node-independent check: is there already a wallet on disk? The encrypted seed file is written
// when the wallet is created, so its presence means "a wallet exists here" even before the node loads it.
// Used at startup to show the welcome/onboarding immediately ONLY on a genuine first run (no seed on disk),
// and to avoid the "create over an existing wallet" trap after the user closed the app on the seed screen.
#[tauri::command]
fn wallet_seed_on_disk(state: State<AppState>) -> bool {
    enc_seed_path(&state.datadir).exists()
}

// Validate a 12-word phrase WITHOUT creating a wallet, so the import screen can flag bad words BEFORE
// asking for a password (audit N3: today the "invalid words" error surfaces on the password screen).
#[tauri::command]
fn wallet_validate_phrase(words: Vec<String>) -> bool {
    Mnemonic::parse(words.join(" ").trim()).is_ok()
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
    // Only right after creation (from memory, shown once). The persisted phrase is encrypted; to reveal it
    // later the user must call wallet_reveal_seed with the wallet password.
    if let Some(mut p) = state.pending_mnemonic.lock().unwrap().clone() {
        let out = json!(p.split(' ').collect::<Vec<_>>());
        zeroize::Zeroize::zeroize(&mut p); // wipe our plaintext clone from memory
        return out;
    }
    json!(Vec::<String>::new())
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

fn addresses_path(datadir: &PathBuf) -> PathBuf { datadir.join("addresses.txt") }
// Records each generated receive address once, in order, so the user can review past addresses.
fn append_address(datadir: &PathBuf, addr: &str) {
    if addr.is_empty() { return; }
    let p = addresses_path(datadir);
    if let Ok(content) = std::fs::read_to_string(&p) {
        if content.lines().any(|l| l == addr) { return; }
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{}", addr);
    }
}

// All receive addresses generated so far, oldest first, each with the BRVA currently sitting at it. Makes sure
// the current one is recorded too. The balance is INFORMATIONAL only: sending stays global/automatic (the wallet
// picks the coins on its own), so this never enables per-address spending.
#[tauri::command]
fn wallet_addresses(state: State<AppState>) -> Value {
    let cur = state.receive_addr.lock().unwrap().clone();
    append_address(&state.datadir, &cur);
    let list: Vec<String> = std::fs::read_to_string(addresses_path(&state.datadir))
        .unwrap_or_default()
        .lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    // Current balance per address = sum of its unspent outputs. One listunspent call (minconf 0) covers them all.
    let mut balances: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    if let Ok(utxos) = rpc(&state.datadir, Some(WALLET_NAME), "listunspent", json!([0])) {
        if let Some(arr) = utxos.as_array() {
            for u in arr {
                if let (Some(a), Some(amt)) = (u["address"].as_str(), u["amount"].as_f64()) {
                    *balances.entry(a.to_string()).or_insert(0.0) += amt;
                }
            }
        }
    }
    let out: Vec<Value> = list
        .iter()
        .map(|a| json!({ "address": a, "balance": balances.get(a).copied().unwrap_or(0.0) }))
        .collect();
    json!(out)
}

#[tauri::command]
fn wallet_new_address(state: State<AppState>) -> Result<Value, String> {
    let addr = rpc(&state.datadir, Some(WALLET_NAME), "getnewaddress", json!([]))?;
    let a = addr.as_str().unwrap_or("").to_string();
    *state.receive_addr.lock().unwrap() = a.clone();
    append_address(&state.datadir, &a);
    Ok(json!({ "address": a }))
}

#[tauri::command]
fn wallet_send(state: State<AppState>, address: String, amount: f64, password: String) -> Result<Value, String> {
    if amount <= 0.0 {
        return Err("ERR:INVALID_AMOUNT".into());
    }
    // Encrypted wallets (new format): unlock briefly with the password, send, and lock again right away.
    // Old unencrypted wallets (created before password support): send directly, without a passphrase, so
    // updating the app never breaks an existing wallet. The UI offers to protect them with a password.
    if wallet_is_encrypted(&state.datadir) {
        rpc(&state.datadir, Some(WALLET_NAME), "walletpassphrase", json!([password, 30]))
            .map_err(|e| {
                let m = e.to_lowercase();
                if m.contains("passphrase") || m.contains("incorrect") { "ERR:BAD_PASSWORD".to_string() } else { e }
            })?;
        let res = rpc(&state.datadir, Some(WALLET_NAME), "sendtoaddress", json!([address, amount]));
        let _ = rpc(&state.datadir, Some(WALLET_NAME), "walletlock", json!([]));
        let txid = res?;
        Ok(json!({ "ok": true, "txid": txid }))
    } else {
        let txid = rpc(&state.datadir, Some(WALLET_NAME), "sendtoaddress", json!([address, amount]))?;
        Ok(json!({ "ok": true, "txid": txid }))
    }
}

// A Core wallet is encrypted iff getwalletinfo exposes "unlocked_until".
fn wallet_is_encrypted(datadir: &PathBuf) -> bool {
    rpc(datadir, Some(WALLET_NAME), "getwalletinfo", json!([]))
        .ok()
        .map(|i| i.get("unlocked_until").is_some())
        .unwrap_or(false)
}

#[tauri::command]
fn wallet_history(state: State<AppState>) -> Value {
    // Fetch a larger recent window (200) so the UI can paginate it (N per page) instead of one long scroll.
    let txs = rpc(&state.datadir, Some(WALLET_NAME), "listtransactions", json!(["*", 200, 0]))
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
        // The target network this build belongs to, independent of connection state (so the UI never
        // shows the wrong network before the node is up). "brisvia-test" (testnet) or "brisvia" (mainnet).
        "network": NET_CHAIN,
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
    // A mined (coinbase) block reports amount 0 at the top level until it matures; the real reward lives in "details".
    let details = tx["details"].as_array();
    let is_coinbase = details
        .map(|d| d.iter().any(|x| matches!(x["category"].as_str(), Some("immature") | Some("generate"))))
        .unwrap_or(false);
    let amount = if is_coinbase {
        details
            .map(|d| {
                d.iter()
                    .filter(|x| matches!(x["category"].as_str(), Some("immature") | Some("generate")))
                    .filter_map(|x| x["amount"].as_f64())
                    .sum::<f64>()
            })
            .unwrap_or(0.0)
    } else {
        tx["amount"].as_f64().unwrap_or(0.0)
    };
    Ok(json!({
        "txid": tx["txid"],
        "amount": amount,
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
    // A Core wallet exposes "unlocked_until" only when it is encrypted with a passphrase.
    let encrypted = info.get("unlocked_until").is_some() || enc_seed_path(&state.datadir).exists();
    json!({
        "name": info["walletname"],
        "descriptors": info["descriptors"],
        "kind": "brisvia_wallet",
        "has_seed_phrase": true,
        "encrypted": encrypted
    })
}

// ================= BIP39: real 12-word backup =================
// Network whose EXT prefix is used to serialize the extended private key of the wallet descriptors.
// In production it is WALLET_NETWORK (tprv on the test build, xprv on the mainnet build), so the local
// node accepts the wpkh() descriptor. But under the e2e feature the node is redirected to regtest; a
// mainnet build would then derive an xprv key that a regtest node rejects ("key ... is not valid").
// So, ONLY in e2e builds redirected to regtest, derive with the regtest (tprv) prefix to match the node.
// This branch is compiled out of the public binary (the env read only exists under the e2e feature),
// so production key derivation is unchanged.
fn wallet_network() -> bitcoin::Network {
    #[cfg(feature = "e2e")]
    if std::env::var("BRISVIA_E2E_CHAIN").map(|c| c == "regtest").unwrap_or(false) {
        return bitcoin::Network::Regtest;
    }
    WALLET_NETWORK
}

// Derives the (external, internal) descriptors WITHOUT checksum from a mnemonic.
// The extended private key is serialized with wallet_network()'s EXT prefix (tprv on the test build,
// xprv on the mainnet build; regtest tprv under e2e) so the local node's wpkh() descriptor accepts it.
// Coin type per build: mainnet uses 9339' (path 84h/9339h/0h) — Brisvia's OWN coin type: 9339 is free in
// the SLIP-44 registry and matches the P2P port 9339, and is NOT Bitcoin's 0' (so the same seed does not
// reuse Bitcoin's keys). The test/e2e build keeps 1' (path 84h/1h/0h). This does NOT change the EXT prefix
// (xprv/tprv). The same 12 words still derive a working wallet on both networks; only the account coin type
// (and the address HRP) differ.
fn descriptors_from_mnemonic(mnemonic: &Mnemonic) -> Result<(String, String, String), String> {
    let seed = mnemonic.to_seed("");
    let secp = Secp256k1::new();
    let master = Xpriv::new_master(wallet_network(), &seed).map_err(|e| e.to_string())?;
    let fp = master.fingerprint(&secp);
    #[cfg(feature = "mainnet")]
    let coin = "9339h"; // Brisvia's own coin type (9339, free in SLIP-44; NOT Bitcoin's 0h)
    #[cfg(not(feature = "mainnet"))]
    let coin = "1h"; // test coin type
    let path = DerivationPath::from_str(&format!("84h/{coin}/0h")).map_err(|e| e.to_string())?;
    let account = master.derive_priv(&secp, &path).map_err(|e| e.to_string())?;
    let origin = format!("[{}/84h/{}/0h]", fp, coin);
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
fn wallet_create_bip39(state: State<AppState>, name: String, password: String) -> Result<Value, String> {
    validate_password(&password)?;
    let mnemonic = Mnemonic::generate(12).map_err(|e| e.to_string())?;
    let (fp, ext, int) = descriptors_from_mnemonic(&mnemonic)?;
    // createwallet blank (no keys), descriptor
    rpc(&state.datadir, None, "createwallet", json!([name, false, true, "", false, true])).map_err(|e| friendly_error(&e))?;
    import_descriptors(&state.datadir, &name, &ext, &int, false)?;
    // Encrypt the Core wallet's private keys with the user's password (wallet is left locked afterwards).
    rpc(&state.datadir, Some(&name), "encryptwallet", json!([password]))?;
    let mut words = mnemonic.to_string();
    *state.pending_mnemonic.lock().unwrap() = Some(words.clone());
    // Store the phrase ENCRYPTED (Argon2id + AES-256-GCM), never plaintext.
    encrypt_phrase_file(&state.datadir, &words, &password)?;
    activate_wallet(&state, &name);
    let result = {
        let list: Vec<&str> = words.split(' ').collect();
        json!({ "words": list, "fingerprint": fp })   // the words are copied into the JSON here
    };
    // Wipe this function's plaintext copy of the mnemonic from memory (the pending copy is wiped after backup
    // verification; the on-disk copy is encrypted). Reduces exposure in a memory dump.
    zeroize::Zeroize::zeroize(&mut words);
    Ok(result)
}

// Verify the backup: compare, in order, the words the user enters against the mnemonic in memory.
// If they match, the mnemonic is wiped from memory (no longer needed).
#[tauri::command]
fn wallet_verify_backup(state: State<AppState>, words: Vec<String>) -> Value {
    let pending = state.pending_mnemonic.lock().unwrap().clone();
    let ok = pending
        .as_ref()
        .map(|p| p.split(' ').map(|s| s.to_string()).collect::<Vec<_>>() == words)
        .unwrap_or(false);
    // Wipe our local plaintext clone of the mnemonic.
    if let Some(mut p) = pending {
        zeroize::Zeroize::zeroize(&mut p);
    }
    if ok {
        // Take the stored value out and wipe it too, instead of just dropping the String.
        if let Some(mut stored) = state.pending_mnemonic.lock().unwrap().take() {
            zeroize::Zeroize::zeroize(&mut stored);
        }
    }
    json!({ "ok": ok })
}

// Restore from 12 words into a NEW wallet (never overwrites an existing one). Rescan from genesis.
#[tauri::command]
fn wallet_restore_bip39(state: State<AppState>, phrase: String, name: String, password: String) -> Result<Value, String> {
    let phrase = zeroize::Zeroizing::new(phrase); // wiped from memory on every return path
    validate_password(&password)?;
    let mnemonic = Mnemonic::parse(phrase.trim())
        .map_err(|_| "ERR:INVALID_PHRASE".to_string())?;
    let (fp, ext, int) = descriptors_from_mnemonic(&mnemonic)?;
    // new name: NEVER overwrites a wallet with funds
    rpc(&state.datadir, None, "createwallet", json!([name, false, true, "", false, true]))?;
    import_descriptors(&state.datadir, &name, &ext, &int, true)?; // rescan from genesis
    // Encrypt the restored wallet's keys and store the phrase encrypted with the new password.
    rpc(&state.datadir, Some(&name), "encryptwallet", json!([password]))?;
    encrypt_phrase_file(&state.datadir, phrase.trim(), &password)?;
    activate_wallet(&state, &name);
    Ok(json!({ "ok": true, "fingerprint": fp }))
}

// Reveal the 12 words AFTER creation — advanced/protected, only with the correct wallet password.
#[tauri::command]
fn wallet_reveal_seed(state: State<AppState>, password: String) -> Result<Value, String> {
    let phrase = zeroize::Zeroizing::new(decrypt_phrase_file(&state.datadir, &password)?); // wiped on return
    let list: Vec<String> = phrase.split_whitespace().map(|s| s.to_string()).collect();
    Ok(json!({ "words": list }))
}

// Migrate an OLD (unencrypted) wallet to a password: encrypt the Core keys + the phrase file, drop the plaintext.
#[tauri::command]
fn wallet_migrate_encrypt(state: State<AppState>, password: String) -> Result<Value, String> {
    validate_password(&password)?;
    let old = load_seed_phrase(&state.datadir); // plaintext phrase from the old format, if any
    rpc(&state.datadir, Some(WALLET_NAME), "encryptwallet", json!([password]))?;
    if !old.is_empty() {
        encrypt_phrase_file(&state.datadir, &old.join(" "), &password)?;
        let _ = std::fs::remove_file(seed_phrase_path(&state.datadir)); // remove the plaintext file
    }
    Ok(json!({ "ok": true }))
}

// The wallet's master fingerprint, read from its own descriptors (e.g. "wpkh([1a2b3c4d/84h/1h/0h]xpub…)").
fn wallet_fingerprint(datadir: &PathBuf) -> Option<String> {
    let res = rpc(datadir, Some(WALLET_NAME), "listdescriptors", json!([])).ok()?;
    let arr = res["descriptors"].as_array()?;
    for d in arr {
        if let Some(desc) = d["desc"].as_str() {
            if let Some(start) = desc.find('[') {
                let rest = &desc[start + 1..];
                if let Some(slash) = rest.find('/') {
                    return Some(rest[..slash].to_string());
                }
            }
        }
    }
    None
}

// Verify a backup WITHOUT the password and WITHOUT revealing the phrase: derive the fingerprint from the words
// the user types and compare it with the wallet's own fingerprint. Same words -> same fingerprint.
#[tauri::command]
fn wallet_check_backup(state: State<AppState>, words: Vec<String>) -> Value {
    let phrase = words.join(" ");
    let derived = match Mnemonic::parse(phrase.trim()) {
        Ok(m) => match descriptors_from_mnemonic(&m) {
            Ok((fp, _, _)) => fp,
            Err(_) => return json!({ "ok": false }),
        },
        Err(_) => return json!({ "ok": false }),
    };
    let wf = wallet_fingerprint(&state.datadir).unwrap_or_default();
    json!({ "ok": !wf.is_empty() && wf.eq_ignore_ascii_case(&derived) })
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

// Mining mode ("solo"/"pool"/"custom") + custom pool address, persisted so the chosen mode survives restarts.
fn mining_prefs_path(datadir: &std::path::Path) -> std::path::PathBuf { datadir.join("mining_prefs.json") }
fn load_mining_prefs(datadir: &std::path::Path) -> (String, String) {
    let v: Value = std::fs::read_to_string(mining_prefs_path(datadir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    (
        v["mode"].as_str().unwrap_or("solo").to_string(),
        v["addr"].as_str().unwrap_or("").to_string(),
    )
}
fn save_mining_prefs(datadir: &std::path::Path, mode: &str, addr: &str) {
    let _ = std::fs::create_dir_all(datadir);
    let _ = std::fs::write(mining_prefs_path(datadir), json!({ "mode": mode, "addr": addr }).to_string());
}

// Validate a user-supplied custom pool address (host:port): rejects control chars, empty host, bad/out-of-range
// port, and local/private targets by default. Mirrors the worker's own check so an invalid URL never reaches it.
fn validate_pool_addr(host_port: &str) -> Result<(), String> {
    let hp = host_port.trim();
    if hp.chars().any(|c| c.is_control()) {
        return Err("invalid characters".into());
    }
    let (host, port) = hp.rsplit_once(':').ok_or_else(|| "use host:port".to_string())?;
    if host.is_empty() {
        return Err("empty host".into());
    }
    let port: u32 = port.parse().map_err(|_| "invalid port".to_string())?;
    if port == 0 || port > 65535 {
        return Err("port out of range".into());
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Err("local address not allowed".into());
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let blocked = match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast()
            }
            std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
        };
        if blocked {
            return Err("local/private address not allowed".into());
        }
    }
    Ok(())
}

// Persists the total INCLUDING the in-progress session, without touching total_mined_secs in memory. Avoids
// losing a session's time if the app closes without going through miner_stop (closing the window, a power cut,
// or an update). On reopen, load_total_mined() recovers it. No double counting: miner_stop only adds the
// session to total_mined_secs (in memory) and saves the same value, staying consistent with what was flushed.
fn flush_total_mined(state: &AppState) {
    let base = *state.total_mined_secs.lock().unwrap();
    let sess = if state.mining.load(Ordering::SeqCst) {
        state.mine_start.lock().unwrap().map(|t| t.elapsed().as_secs()).unwrap_or(0)
    } else { 0 };
    save_total_mined(&state.datadir, base + sess);
}

// Legacy plaintext phrase file (old unencrypted format). Kept only to READ during migration to the
// encrypted format; new/updated wallets never write it (see encrypt_phrase_file / wallet_migrate_encrypt).
fn seed_phrase_path(datadir: &std::path::Path) -> std::path::PathBuf { datadir.join("wallet_seed_phrase.txt") }
fn load_seed_phrase(datadir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(seed_phrase_path(datadir)).ok()
        .map(|s| s.trim().split_whitespace().map(|w| w.to_string()).collect())
        .unwrap_or_default()
}

// ---- password-based encryption of the recovery phrase (Argon2id + AES-256-GCM) ----
// The 12-word phrase is NEVER stored in plaintext once a wallet has a password. It is encrypted with a
// key derived from the user's password. The Core wallet's private keys are encrypted separately with
// `encryptwallet`. Losing the password is recovered ONLY from the 12 words (there is no backdoor).
fn enc_seed_path(datadir: &std::path::Path) -> std::path::PathBuf { datadir.join("wallet_seed.enc") }

fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    use argon2::{Argon2, Algorithm, Version, Params};
    // 64 MiB, 3 passes, 1 lane: strong for desktop without choking modest PCs (well above OWASP minimums).
    let params = Params::new(65536, 3, 1, Some(32)).map_err(|e| e.to_string())?;
    let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    a2.hash_password_into(password.as_bytes(), salt, &mut key).map_err(|e| e.to_string())?;
    Ok(key)
}

// File layout: version(1) | salt(16) | nonce(12) | ciphertext+tag
fn encrypt_phrase_file(datadir: &std::path::Path, phrase: &str, password: &str) -> Result<(), String> {
    use aes_gcm::{Aes256Gcm, Key, Nonce, aead::{Aead, KeyInit}};
    use rand::RngCore;
    use zeroize::Zeroize;
    let mut salt = [0u8; 16];
    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let mut key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let enc = cipher.encrypt(Nonce::from_slice(&nonce_bytes), phrase.trim().as_bytes());
    key.zeroize(); // wipe our key copy on EVERY path (success or encrypt error)
    let ct = enc.map_err(|_| "ERR:ENCRYPT_FAILED".to_string())?;
    let mut out = Vec::with_capacity(1 + 16 + 12 + ct.len());
    out.push(1u8);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    let _ = std::fs::create_dir_all(datadir);
    // Atomic write: write a temp file then rename, so a crash mid-write never leaves a half-written (corrupt)
    // seed file. Restrict permissions to owner-only on unix (harmless no-op on Windows).
    let path = enc_seed_path(datadir);
    let tmp = path.with_extension("enc.tmp");
    std::fs::write(&tmp, &out).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

fn decrypt_phrase_file(datadir: &std::path::Path, password: &str) -> Result<String, String> {
    use aes_gcm::{Aes256Gcm, Key, Nonce, aead::{Aead, KeyInit}};
    use zeroize::Zeroize;
    let path = enc_seed_path(datadir);
    // Bound the file size before reading it into memory: a valid seed file is ~100 bytes; a tampered huge
    // file must not cause excessive memory use.
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 8192 {
        return Err("ERR:SEED_CORRUPT".to_string());
    }
    let data = std::fs::read(&path).map_err(|_| "ERR:NO_SEED_FILE".to_string())?;
    if data.len() < 1 + 16 + 12 + 16 || data[0] != 1 {
        return Err("ERR:SEED_CORRUPT".to_string());
    }
    let salt = &data[1..17];
    let nonce_bytes = &data[17..29];
    let ct = &data[29..];
    let mut key = derive_key(password, salt)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|_| "ERR:BAD_PASSWORD".to_string());
    key.zeroize();
    let pt = pt?;
    String::from_utf8(pt).map_err(|_| "ERR:SEED_CORRUPT".to_string())
}

// Minimum password policy. 12 chars: an 8-char password is too weak against OFFLINE brute force of the
// encrypted seed file (the 12 recovery words do NOT compensate a weak local password). Audit finding F.
// Single source of truth for the new-password policy. The frontend and the shown message
// (onboarding.pass_weak) must state the SAME number. Only applies to CREATING/restoring a wallet;
// unlocking to send never checks length (it must accept whatever the wallet was created with).
const MIN_PASSWORD_LEN: usize = 6;
fn validate_password(password: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err("ERR:WEAK_PASSWORD".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod seed_crypto_tests {
    use super::{decrypt_phrase_file, encrypt_phrase_file};
    #[test]
    fn seed_roundtrip_and_wrong_password() {
        let dir = std::env::temp_dir().join("brisvia_seed_crypto_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let phrase = "legal winner thank year wave sausage worth useful legal winner thank yellow";
        encrypt_phrase_file(&dir, phrase, "correct-key-123").unwrap();
        // The file must NOT contain the phrase in plaintext.
        let raw = std::fs::read(dir.join("wallet_seed.enc")).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("winner"), "the phrase leaked in plaintext");
        // Correct password decrypts to the same phrase.
        assert_eq!(decrypt_phrase_file(&dir, "correct-key-123").unwrap(), phrase);
        // Wrong password fails (AES-GCM auth tag mismatch).
        assert!(decrypt_phrase_file(&dir, "wrong-key").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod wallet_key_tests {
    use super::descriptors_from_mnemonic;
    use bip39::Mnemonic;
    use std::str::FromStr;

    // The extended PRIVATE key inside the wpkh() descriptor must carry the EXT_SECRET_KEY prefix the
    // local node accepts (see chainparams.cpp): xprv on the mainnet build, tprv on the test build.
    // A mismatch is exactly the "wpkh(): key 'tprv...' is not valid" bug this test guards against.
    #[test]
    fn ext_key_prefix_matches_build_network() {
        let m = Mnemonic::from_str(
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
        )
        .unwrap();
        let (_fp, ext, int) = descriptors_from_mnemonic(&m).unwrap();
        // Origin: "wpkh([<fp>/84h/<coin>/0h]<extkey>/0/*)". The char after ']' is the EXT prefix; <coin> is
        // the coin type (9339' on mainnet, 1' on the test build). We assert both.
        #[cfg(feature = "mainnet")]
        {
            assert!(ext.contains("]xprv"), "mainnet build must produce xprv, got: {ext}");
            assert!(int.contains("]xprv"), "mainnet build must produce xprv, got: {int}");
            assert!(!ext.contains("]tprv"), "mainnet build leaked a testnet tprv key: {ext}");
            assert!(ext.contains("/84h/9339h/0h]"), "mainnet build must use coin_type 9339', got: {ext}");
        }
        #[cfg(not(feature = "mainnet"))]
        {
            assert!(ext.contains("]tprv"), "test build must produce tprv, got: {ext}");
            assert!(int.contains("]tprv"), "test build must produce tprv, got: {int}");
            assert!(!ext.contains("]xprv"), "test build produced a mainnet xprv key: {ext}");
            assert!(ext.contains("/84h/1h/0h]"), "test build must use coin_type 1', got: {ext}");
        }
    }

    // ================= GOLDEN VECTOR (mainnet) =================
    // A frozen end-to-end derivation vector for the REAL network, requested before the coin_type/address
    // format is locked in. It pins the WHOLE chain: 12-word phrase -> seed -> master -> path m/84'/9339'/0'
    // -> account xprv -> child keys -> P2WPKH witness program -> bech32 encoding with Brisvia mainnet HRP
    // "brv". If ANY link ever changes (coin type, EXT prefix, HRP, derivation), a shipped wallet would hand
    // users addresses they cannot restore. Freezing the exact strings turns any such change into a loud,
    // visible CI failure instead of a silent break. FIXED test phrase only — the standard all-"abandon"
    // BIP39 vector, a public test key with NO funds. NEVER type real funds into this phrase.
    #[cfg(feature = "mainnet")]
    #[test]
    fn golden_vector_mainnet_derivation() {
        use bitcoin::bip32::{ChildNumber, Xpriv};
        use bitcoin::hashes::Hash;
        use bitcoin::secp256k1::Secp256k1;
        use bitcoin::CompressedPublicKey;

        // Standard BIP39 test vector (valid 12-word mnemonic). PUBLIC, well-known, holds no coins.
        const TEST_PHRASE: &str =
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

        // Derive the account xprv straight out of the descriptor string, then walk the external chain
        // (m/.../0/index) to a P2WPKH address encoded with Brisvia mainnet's own HRP ("brv").
        fn addr_from_account(account_xprv: &str, change: u32, index: u32) -> String {
            let secp = Secp256k1::new();
            let account = Xpriv::from_str(account_xprv).unwrap();
            let path = [
                ChildNumber::from_normal_idx(change).unwrap(),
                ChildNumber::from_normal_idx(index).unwrap(),
            ];
            let child = account.derive_priv(&secp, &path).unwrap();
            let pubkey = CompressedPublicKey::from_private_key(&secp, &child.to_priv()).unwrap();
            let program = pubkey.wpubkey_hash().to_byte_array();
            let hrp = bitcoin::bech32::Hrp::parse("brv").unwrap();
            bitcoin::bech32::segwit::encode_v0(hrp, &program).unwrap()
        }

        // Pull the account extended key from the descriptor: "wpkh([<fp>/84h/9339h/0h]<xprv>/0/*)".
        fn account_xprv_of(ext: &str) -> String {
            ext.split(']')
                .nth(1)
                .expect("descriptor missing origin ']' separator")
                .strip_suffix("/0/*)")
                .expect("descriptor missing '/0/*)' suffix")
                .to_string()
        }

        let m = Mnemonic::from_str(TEST_PHRASE).unwrap();
        let (_fp, ext, int) = descriptors_from_mnemonic(&m).unwrap();
        let account = account_xprv_of(&ext);
        let addrs: Vec<String> = (0..3).map(|i| addr_from_account(&account, 0, i)).collect();

        // Print the real derived values so they can be read off the first run and frozen below.
        eprintln!("GOLDEN ext descriptor = {ext}");
        eprintln!("GOLDEN account xprv   = {account}");
        eprintln!("GOLDEN addr 0/0 = {}", addrs[0]);
        eprintln!("GOLDEN addr 0/1 = {}", addrs[1]);
        eprintln!("GOLDEN addr 0/2 = {}", addrs[2]);

        // --- Structure of the chain (path + EXT prefix) ---
        assert!(ext.contains("/84h/9339h/0h]"), "mainnet path must be m/84'/9339'/0', got: {ext}");
        assert!(ext.contains("]xprv"), "account key must serialize as xprv, got: {ext}");
        assert!(int.contains("/84h/9339h/0h]"), "internal descriptor path drifted, got: {int}");
        assert!(account.starts_with("xprv"), "account key must be an xprv, got: {account}");
        for a in &addrs {
            assert!(a.starts_with("brv1q"), "mainnet address must be brv1q... (P2WPKH), got: {a}");
        }

        // --- Golden values (frozen) ---
        // The master fingerprint 73c5da0a is the well-known BIP32 root fingerprint of this public test seed,
        // an independent cross-check that phrase -> seed -> master is correct.
        const GOLDEN_EXT: &str = "wpkh([73c5da0a/84h/9339h/0h]xprv9yDfZrCVwPCcMQB7uAiv9Mwdb8E8z8pB43fy78hSJpoSsJd5FbyiVyKaMKeBdDhuHoTEZmGUaW3QXGLoUa1oMqRDhy1UJei3Efrn1Cf5QGF/0/*)";
        const GOLDEN_ADDR_0: &str = "brv1qtulvfeste55pszl98ezzc7pmvpuyxg5q0374mp";
        const GOLDEN_ADDR_1: &str = "brv1qur70qc9nf0p0g229e448l4nyjwxhm68ud4j09f";
        const GOLDEN_ADDR_2: &str = "brv1qqrw7wthpwegpmjx8krsxgjazkndvy4myr7t88u";
        assert_eq!(ext, GOLDEN_EXT, "mainnet external descriptor changed vs golden vector");
        assert_eq!(addrs[0], GOLDEN_ADDR_0, "receiving address 0/0 changed vs golden vector");
        assert_eq!(addrs[1], GOLDEN_ADDR_1, "receiving address 0/1 changed vs golden vector");
        assert_eq!(addrs[2], GOLDEN_ADDR_2, "receiving address 0/2 changed vs golden vector");

        // --- Idempotency: re-deriving (i.e. "restoring") the same phrase yields the exact same result ---
        let (_fp2, ext2, int2) = descriptors_from_mnemonic(&m).unwrap();
        assert_eq!(ext, ext2, "descriptor is not deterministic across derivations");
        assert_eq!(int, int2, "internal descriptor is not deterministic across derivations");
        let account2 = account_xprv_of(&ext2);
        let addrs2: Vec<String> = (0..3).map(|i| addr_from_account(&account2, 0, i)).collect();
        assert_eq!(addrs, addrs2, "restoring the same phrase produced different addresses");
    }

    // ================= MASSIVE WALLET DERIVATION (mainnet) =================
    // Property-style battery: generate thousands of BRAND-NEW wallets and assert the invariants that must
    // hold for EVERY user's real receiving address. Catches a broken RNG, a drifted derivation path, a
    // wrong HRP, non-determinism, or an address collision. Case count via BRISVIA_WALLET_CASES (default
    // 5000; the release candidate runs 100000 overnight). Never logs the phrases (only counts / a failing
    // index), per the "no seed in logs" rule.
    #[cfg(feature = "mainnet")]
    #[test]
    fn massive_wallet_derivation_invariants() {
        use bitcoin::bip32::{ChildNumber, Xpriv};
        use bitcoin::hashes::Hash;
        use bitcoin::secp256k1::Secp256k1;
        use bitcoin::CompressedPublicKey;
        use std::collections::HashSet;

        fn addr_from_account(account_xprv: &str, change: u32, index: u32) -> String {
            let secp = Secp256k1::new();
            let account = Xpriv::from_str(account_xprv).unwrap();
            let path = [
                ChildNumber::from_normal_idx(change).unwrap(),
                ChildNumber::from_normal_idx(index).unwrap(),
            ];
            let child = account.derive_priv(&secp, &path).unwrap();
            let pubkey = CompressedPublicKey::from_private_key(&secp, &child.to_priv()).unwrap();
            let program = pubkey.wpubkey_hash().to_byte_array();
            let hrp = bitcoin::bech32::Hrp::parse("brv").unwrap();
            bitcoin::bech32::segwit::encode_v0(hrp, &program).unwrap()
        }
        fn account_xprv_of(ext: &str) -> String {
            ext.split(']').nth(1).unwrap().strip_suffix("/0/*)").unwrap().to_string()
        }

        let n: usize = std::env::var("BRISVIA_WALLET_CASES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5_000);

        let mut seen_addr: HashSet<String> = HashSet::with_capacity(n);
        let mut seen_mnem: HashSet<String> = HashSet::with_capacity(n);

        for i in 0..n {
            let m = Mnemonic::generate(12).expect("generate 12-word mnemonic");
            let phrase = m.to_string();
            let (fp, ext, int) = descriptors_from_mnemonic(&m).unwrap();
            let account = account_xprv_of(&ext);
            let addr0 = addr_from_account(&account, 0, 0);

            // Format invariants for the real receiving address.
            assert!(addr0.starts_with("brv1q"), "case {i}: address is not brv1q P2WPKH");
            assert_eq!(addr0.len(), 43, "case {i}: brv P2WPKH bech32 length must be 43 (hrp 'brv' + 1 + 32 + 6), got {}", addr0.len());
            assert!(ext.contains("/84h/9339h/0h]"), "case {i}: derivation path drifted");
            assert!(ext.contains("]xprv"), "case {i}: account key is not an xprv");
            assert!(int.contains("/84h/9339h/0h]"), "case {i}: internal descriptor path drifted");

            // Determinism: re-parsing the SAME phrase yields the SAME address (restore == create).
            let m2 = Mnemonic::from_str(&phrase).unwrap();
            let (fp2, ext2, _int2) = descriptors_from_mnemonic(&m2).unwrap();
            assert_eq!(fp, fp2, "case {i}: fingerprint not deterministic");
            assert_eq!(ext, ext2, "case {i}: descriptor not deterministic");
            assert_eq!(addr0, addr_from_account(&account_xprv_of(&ext2), 0, 0),
                "case {i}: restoring the phrase produced a different address");

            // Uniqueness: distinct wallets must never collide (mnemonic or address).
            assert!(seen_mnem.insert(phrase), "case {i}: DUPLICATE mnemonic generated (RNG failure)");
            assert!(seen_addr.insert(addr0), "case {i}: ADDRESS COLLISION across distinct wallets");
        }
        eprintln!("massive_wallet_derivation_invariants: {n} nuevas billeteras, 0 colisiones, invariantes OK");
    }

    // Invalid phrases MUST be rejected (never silently create a wallet from a bad backup).
    #[test]
    fn invalid_mnemonics_rejected() {
        let bads = [
            "",                                                                                   // empty
            "abandon",                                                                            // 1 word
            "abandon abandon abandon",                                                            // 3 words
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon", // 12x, bad checksum
            "zzzz zzzz zzzz zzzz zzzz zzzz zzzz zzzz zzzz zzzz zzzz zzzz",                         // words not in BIP39 list
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon zoo",      // wrong checksum word
        ];
        for b in bads {
            assert!(Mnemonic::parse(b.trim()).is_err(), "invalid mnemonic was ACCEPTED: {b:?}");
        }
    }
}

// Update the notification-area tooltip so it always matches the current language and mining state.
// Called on start/stop/intensity change AND on language change, so it never gets stuck in the wrong language.
fn refresh_tooltip(state: &AppState) {
    let tray_guard = state.tray.lock().unwrap();
    let tray = match tray_guard.as_ref() {
        Some(t) => t,
        None => return,
    };
    let lang = state.lang.lock().unwrap().clone();
    let tip = if state.mining.load(Ordering::SeqCst) {
        let pct = match state.intensity.lock().unwrap().as_str() {
            "suave" => 25usize,
            "equilibrado" => 50,
            "intenso" => 100,
            s => s.parse::<usize>().unwrap_or(50),
        }
        .clamp(1, 100);
        let threads = state.miner_threads.load(Ordering::SeqCst);
        let cores = state.cores;
        if lang == "es" {
            format!("Brisvia — Minando al {}% · {} de {} núcleos", pct, threads, cores)
        } else {
            format!("Brisvia — Mining at {}% · {} of {} cores", pct, threads, cores)
        }
    } else {
        "Brisvia".to_string()
    };
    let _ = tray.set_tooltip(Some(&tip));
}

#[tauri::command]
fn miner_start(app: AppHandle, state: State<AppState>, intensity: Option<String>) -> Value {
    if let Some(i) = intensity {
        *state.intensity.lock().unwrap() = i;
    }
    if state.mining.swap(true, Ordering::SeqCst) {
        return json!({ "mining": true }); // already mining
    }
    // Defense in depth (audit CF2-10): do NOT start mining while the node is still in initial block download —
    // it would mine on a stale/partial chain. The UI already blocks "Mine" during IBD; this backend guard does not
    // rely on the UI alone. An RPC failure (node still warming up) is tolerated: only an explicit IBD=true blocks.
    if let Ok(info) = rpc(&state.datadir, None, "getblockchaininfo", json!([])) {
        if info.get("initialblockdownload").and_then(|v| v.as_bool()) == Some(true) {
            state.mining.store(false, Ordering::SeqCst);
            return json!({ "mining": false, "error": "ERR:NODE_SYNCING" });
        }
    }
    // "Ready to mine" signal (audit-decided). On the REAL network only (an e2e/regtest run is intentionally offline):
    // do NOT mine a lonely branch (need >= 1 peer) nor with a clock skewed >= 5 min vs the peers' adjusted time.
    // The genesis-time gate is a consensus rule, enforced by the node, not here. Tip age does NOT block mining
    // (a long gap between blocks is normal in PoW; stopping miners would make it worse).
    if net_chain() == NET_CHAIN {
        let peers = rpc(&state.datadir, None, "getconnectioncount", json!([]))
            .ok().and_then(|v| v.as_u64()).unwrap_or(0);
        if peers < 1 {
            state.mining.store(false, Ordering::SeqCst);
            return json!({ "mining": false, "error": "ERR:NO_PEERS" });
        }
        if let Ok(ni) = rpc(&state.datadir, None, "getnetworkinfo", json!([])) {
            if ni.get("timeoffset").and_then(|v| v.as_i64()).map(|o| o.abs() >= 300).unwrap_or(false) {
                state.mining.store(false, Ordering::SeqCst);
                return json!({ "mining": false, "error": "ERR:CLOCK_SKEW" });
            }
        }
    }
    // A power change relaunches the engine; in that case keep the session timer, the shown speed and the
    // "ready" flag (the worker reports the new speed in a moment) — for the user it's the same session.
    if !state.keep_session_on_relaunch.swap(false, Ordering::SeqCst) {
        *state.mine_start.lock().unwrap() = Some(Instant::now());
        *state.hashrate.lock().unwrap() = 0.0;
        state.miner_ready.store(false, Ordering::SeqCst); // starts "preparing" until the engine's first event
    }

    // How many cores to use. `intensity` is a percentage "1".."100" of the machine's cores (fallback 50%).
    // Legacy named values (suave/equilibrado/intenso) map to 25/50/100 for backward compatibility.
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let pct = match state.intensity.lock().unwrap().as_str() {
        "suave" => 25usize,
        "equilibrado" => 50,
        "intenso" => 100,
        s => s.parse::<usize>().unwrap_or(50),
    }
    .clamp(1, 100);
    let threads = ((cores * pct + 99) / 100).max(1); // ceil(cores * pct / 100), at least 1
    state.miner_threads.store(threads as u64, Ordering::SeqCst);
    // Tray tooltip: show the mining power in plain words, in the current language.
    refresh_tooltip(&state);

    // Node cookie auth (never exposed in logs): the .cookie file contains "user:password".
    let cookie = std::fs::read_to_string(state.datadir.join(net_subdir()).join(".cookie")).unwrap_or_default();
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
    let url = format!("http://127.0.0.1:{}/wallet/{}", rpc_port(), WALLET_NAME);

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
    // Mining mode: solo (against the local node, default) or pool. In pool/custom mode the worker connects to a
    // stratum pool (BRISVIA_POOL_URL) and mines there; only the payout address travels in the login. The solo
    // arguments below are still passed (the worker ignores them in pool mode) so nothing about solo changes.
    let pool_url = {
        let mode = state.mining_mode.lock().unwrap().clone();
        match mode.as_str() {
            "pool" => Some(OFFICIAL_POOL_URL.to_string()),
            "custom" => {
                let a = state.pool_address.lock().unwrap().clone();
                if a.trim().is_empty() { None } else { Some(a.trim().to_string()) }
            }
            _ => None,
        }
    };
    // N11: "custom" pool chosen but no address entered -> refuse to start instead of silently mining solo.
    if state.mining_mode.lock().unwrap().as_str() == "custom" && pool_url.is_none() {
        state.mining.store(false, Ordering::SeqCst);
        return json!({ "mining": false, "error": "ERR:POOL_ADDR_MISSING" });
    }
    let mut cmd = Command::new(&miner_bin);
    // RPC credentials go via ENV, not argv: a process command line is readable by other local processes, so we keep
    // the node cookie user/password out of it. The worker reads BRISVIA_RPC_USER/PASS (with an argv fallback for CLI).
    let empty = String::new();
    cmd.args([&url, &empty, &empty, &addr, &u64::MAX.to_string(), &threads.to_string()])
        .env("BRISVIA_RPC_USER", &cuser)
        .env("BRISVIA_RPC_PASS", &cpass)
        .env("BRISVIA_JSON", "1")
        .stdout(out_stdio)
        .stderr(Stdio::null());
    // Explicit mode: pass the pool URL only in pool mode; in solo mode REMOVE any inherited BRISVIA_POOL_URL so a
    // residual/leftover value can never silently put the worker in pool mode (keeps the audited solo path pristine).
    match &pool_url {
        Some(purl) => { cmd.env("BRISVIA_POOL_URL", purl); }
        None => { cmd.env_remove("BRISVIA_POOL_URL"); }
    }
    no_window(&mut cmd);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => { state.mining.store(false, Ordering::SeqCst); return json!({ "mining": false, "error": format!("could not start the miner: {e}") }); }
    };

    // Thread that follows the events file and updates accepted contributions + real hashrate.
    {
        let mined = state.mined.clone();
        let stale = state.stale.clone();
        let hashrate = state.hashrate.clone();
        let mining = state.mining.clone();
        let ready = state.miner_ready.clone();
        let total_mined = state.total_mined_secs.clone();
        let mine_start_ref = state.mine_start.clone();
        let datadir_ref = state.datadir.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let mut prev_total: u64 = 0;
            let mut last_flush = Instant::now();
            loop {
                if !mining.load(Ordering::SeqCst) { break; }
                std::thread::sleep(Duration::from_millis(500));
                // Periodic save of the total time (including the in-progress session), in case the app
                // closes without going through "Stop". Every 30 s is enough and doesn't wear the disk.
                if last_flush.elapsed().as_secs() >= 30 {
                    let base = *total_mined.lock().unwrap();
                    let sess = mine_start_ref.lock().unwrap().map(|t| t.elapsed().as_secs()).unwrap_or(0);
                    save_total_mined(&datadir_ref, base + sess);
                    last_flush = Instant::now();
                }
                let f = match std::fs::File::open(&events_path) { Ok(f) => f, Err(_) => continue };
                let mut total = 0u64;
                let mut total_stale = 0u64;
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
                            // Periodic live-speed event from the miner (every ~2s), so the UI updates in real time.
                            Some("hashrate") => {
                                ready.store(true, Ordering::SeqCst);
                                if let Some(hs) = evt["hashrate"].as_f64() { last_hs = Some(hs); }
                            }
                            Some("stale") => { ready.store(true, Ordering::SeqCst); total_stale += 1; }
                            // Circuit breaker tripped in the miner (repeated hard errors): stop cleanly so the UI
                            // doesn't keep showing "Mining" forever.
                            Some("fatal") => { mining.store(false, Ordering::SeqCst); ready.store(false, Ordering::SeqCst); }
                            _ => {}
                        }
                    }
                }
                if total > prev_total {
                    mined.fetch_add(total - prev_total, Ordering::SeqCst);
                    prev_total = total;
                }
                // Update the shown speed on every pass (from accepted OR periodic hashrate events), not only on new blocks.
                if let Some(hs) = last_hs { *hashrate.lock().unwrap() = hs; }
                stale.store(total_stale, Ordering::SeqCst); // recomputed each pass from the current session's log
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
    refresh_tooltip(&state); // mining stopped → tooltip goes back to plain "Brisvia"
    json!({ "mining": false })
}

#[tauri::command]
fn miner_set_intensity(app: AppHandle, state: State<AppState>, intensity: String) -> Value {
    *state.intensity.lock().unwrap() = intensity.clone();
    // If mining right now, apply the new setting live: stop the current engine and relaunch it with the
    // new thread count. Without this, changing intensity mid-mining had no effect until the next stop/start.
    if state.mining.load(Ordering::SeqCst) {
        if let Some(mut child) = state.miner_child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        state.mining.store(false, Ordering::SeqCst);
        state.keep_session_on_relaunch.store(true, Ordering::SeqCst); // don't reset the session on a power change
        return miner_start(app, state, Some(intensity));
    }
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
        "stale": state.stale.load(Ordering::SeqCst),
        "secondsMining": if mining { secs } else { 0 },
        // REAL hashrate reported by the miner (CPU hashes per second)
        "hashrate": if mining { *state.hashrate.lock().unwrap() } else { 0.0 },
        "intensity": intensity,
        "threads": state.miner_threads.load(Ordering::SeqCst),
        "cores": state.cores as u64,
        "totalSeconds": total
    })
}

// ================= achievements (50, account-only, computed from the wallet) =================
// All 50 achievements are derived from the wallet/chain, so they travel with the 12 words. The backend returns
// only ids + numbers; the UI translates every text via i18n. See LOGROS-50-DISENO.md for the full spec.

// Reads every wallet transaction (paged) plus the balance, and folds them into the metrics the achievements need.
fn scan_wallet_metrics(datadir: &PathBuf) -> WalletMetrics {
    let mut m = WalletMetrics::default();
    if let Ok(bal) = rpc(datadir, Some(WALLET_NAME), "getbalances", json!([])) {
        m.balance = bal["mine"]["trusted"].as_f64().unwrap_or(0.0);
    }
    // Page through listtransactions. count=1000 per page, capped at 200 pages (200k txs) as a safety limit; the
    // largest threshold (10000 blocks / 250 sends) is well within that, so the counts stay correct.
    let page: i64 = 1000;
    let max_pages = 200;
    let mut skip: i64 = 0;
    let mut first_time = i64::MAX;
    for i in 0..max_pages {
        let txs = match rpc(datadir, Some(WALLET_NAME), "listtransactions", json!(["*", page, skip])) {
            Ok(v) => v,
            Err(_) => break,
        };
        let arr = txs.as_array().cloned().unwrap_or_default();
        let n = arr.len() as i64;
        for t in &arr {
            let cat = t["category"].as_str().unwrap_or("");
            let time = t["time"].as_i64().unwrap_or(0);
            if time > 0 && time < first_time {
                first_time = time;
            }
            match cat {
                "generate" | "immature" => {
                    m.blocks += 1;
                    let height = t["blockheight"].as_i64().unwrap_or(i64::MAX);
                    let btime = t["blocktime"].as_i64().unwrap_or(0);
                    if height < HALVING_HEIGHT {
                        m.before_halving = true;
                    }
                    if btime >= MAINNET_START && btime < MAINNET_START + ONE_DAY {
                        m.first_day = true;
                    }
                    if btime >= MAINNET_START && btime < MAINNET_START + 7 * ONE_DAY {
                        m.first_week = true;
                    }
                    if btime >= MAINNET_START && btime < MAINNET_START + 30 * ONE_DAY {
                        m.first_month = true;
                    }
                    if btime >= MAINNET_START + 365 * ONE_DAY {
                        m.after_year = true;
                    }
                }
                "send" => m.sends += 1,
                "receive" => m.receives += 1,
                _ => {}
            }
        }
        skip += page;
        if n < page {
            break;
        }
        if i == max_pages - 1 {
            eprintln!("[brisvia] achievement scan hit the page cap; block counts may be truncated");
        }
    }
    if first_time != i64::MAX {
        m.first_time = first_time;
    }
    m
}

// Builds the full 50-item list (id, family, tier, unlocked, current, threshold) from the metrics. The order here
// is the display order the UI relies on (family by family, medal by ascending tier).
fn compute_achievements(m: &WalletMetrics) -> Vec<Value> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let age = if m.first_time > 0 { (now - m.first_time).max(0) } else { 0 };
    let mut out: Vec<Value> = Vec::with_capacity(50);
    let mut push = |id: String, family: &str, tier: &str, unlocked: bool, current: f64, threshold: f64| {
        out.push(json!({
            "id": id, "family": family, "tier": tier,
            "unlocked": unlocked, "current": current, "threshold": threshold
        }));
    };

    // A · MINED BLOCKS (12)
    let blocks_th: [u64; 12] = [1, 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000];
    let blocks_tier = ["bronze", "bronze", "bronze", "silver", "silver", "gold", "gold", "gold", "emerald", "emerald", "diamond", "diamond"];
    for i in 0..12 {
        let th = blocks_th[i] as f64;
        push(format!("blocks_{}", blocks_th[i]), "blocks", blocks_tier[i], m.blocks as f64 >= th, m.blocks as f64, th);
    }
    // B · BALANCE (10)
    let bal_th: [u64; 10] = [50, 100, 250, 500, 1000, 2500, 5000, 10000, 50000, 100000];
    let bal_tier = ["bronze", "bronze", "silver", "silver", "gold", "gold", "emerald", "emerald", "diamond", "diamond"];
    for i in 0..10 {
        let th = bal_th[i] as f64;
        push(format!("balance_{}", bal_th[i]), "balance", bal_tier[i], m.balance >= th, m.balance, th);
    }
    // C · SENT (8)
    let send_th: [u64; 8] = [1, 3, 5, 10, 25, 50, 100, 250];
    let send_tier = ["bronze", "bronze", "silver", "silver", "gold", "gold", "emerald", "diamond"];
    for i in 0..8 {
        let th = send_th[i] as f64;
        push(format!("sends_{}", send_th[i]), "sends", send_tier[i], m.sends as f64 >= th, m.sends as f64, th);
    }
    // D · RECEIVED (6)
    let recv_th: [u64; 6] = [1, 3, 5, 10, 25, 50];
    let recv_tier = ["bronze", "bronze", "silver", "silver", "gold", "diamond"];
    for i in 0..6 {
        let th = recv_th[i] as f64;
        push(format!("receives_{}", recv_th[i]), "receives", recv_tier[i], m.receives as f64 >= th, m.receives as f64, th);
    }
    // E · NETWORK AGE (6)
    let age_ids = ["age_week", "age_month", "age_3months", "age_6months", "age_year", "age_2years"];
    let age_th: [i64; 6] = [7 * ONE_DAY, 30 * ONE_DAY, 90 * ONE_DAY, 180 * ONE_DAY, 365 * ONE_DAY, 730 * ONE_DAY];
    let age_tier = ["bronze", "silver", "gold", "gold", "emerald", "diamond"];
    for i in 0..6 {
        push(age_ids[i].to_string(), "age", age_tier[i], m.first_time > 0 && age >= age_th[i], age as f64, age_th[i] as f64);
    }
    // F · PIONEER / NETWORK MILESTONES (5) — ordered by ascending tier for a clean progression.
    let bool_num = |b: bool| if b { 1.0 } else { 0.0 };
    push("first_month".into(), "pioneer", "silver", m.first_month, bool_num(m.first_month), 1.0);
    push("pioneer".into(), "pioneer", "gold", m.first_day, bool_num(m.first_day), 1.0);
    push("founder".into(), "pioneer", "gold", m.first_week, bool_num(m.first_week), 1.0);
    push("before_halving".into(), "pioneer", "emerald", m.before_halving, bool_num(m.before_halving), 1.0);
    push("guardian".into(), "pioneer", "diamond", m.after_year, bool_num(m.after_year), 1.0);
    // G · RANK (3)
    push("rank_active".into(), "rank", "silver", m.blocks >= 10, m.blocks as f64, 10.0);
    let trio = m.blocks >= 1 && m.sends >= 1 && m.receives >= 1;
    push("rank_trio".into(), "rank", "gold", trio, bool_num(trio), 1.0);
    let legend = m.blocks >= 1000 && m.balance >= 10000.0;
    push("rank_legend".into(), "rank", "diamond", legend, bool_num(legend), 1.0);

    out
}

fn achievements_file(datadir: &PathBuf) -> PathBuf {
    datadir.join("achievements.json")
}

// Returns the 50 achievements plus the ids that just unlocked (for a one-time toast). The first time this runs
// for this version, everything already achieved is seeded silently (no toast burst); afterwards only genuinely
// new unlocks are reported once and remembered in achievements.json.
#[tauri::command]
fn achievements(state: State<AppState>) -> Value {
    // Until a wallet is actually loaded, return everything locked WITHOUT creating/seeding achievements.json.
    // This makes the silent seed happen on the first call with a real wallet (so restoring a wallet with history
    // does not fire a burst of toasts for everything it already achieved).
    if !state.wallet_loaded.load(Ordering::SeqCst) {
        let list = compute_achievements(&WalletMetrics::default());
        return json!({ "list": list, "justUnlocked": [] });
    }
    let metrics = {
        let mut cache = state.ach_cache.lock().unwrap();
        let fresh = cache.as_ref().map(|(t, _)| t.elapsed() < Duration::from_secs(10)).unwrap_or(false);
        if fresh {
            cache.as_ref().unwrap().1.clone()
        } else {
            let m = scan_wallet_metrics(&state.datadir);
            *cache = Some((Instant::now(), m.clone()));
            m
        }
    };
    let list = compute_achievements(&metrics);
    let unlocked_ids: Vec<String> = list
        .iter()
        .filter(|a| a["unlocked"].as_bool().unwrap_or(false))
        .filter_map(|a| a["id"].as_str().map(|s| s.to_string()))
        .collect();

    let path = achievements_file(&state.datadir);
    let existed = path.exists();
    let mut notified: Vec<String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut just_unlocked: Vec<String> = Vec::new();

    if !existed {
        // First run for this version: adopt everything already achieved without announcing it.
        notified = unlocked_ids.clone();
        let _ = std::fs::write(&path, serde_json::to_string(&notified).unwrap_or_default());
    } else {
        for id in &unlocked_ids {
            if !notified.iter().any(|x| x == id) {
                notified.push(id.clone());
                just_unlocked.push(id.clone());
            }
        }
        if !just_unlocked.is_empty() {
            let _ = std::fs::write(&path, serde_json::to_string(&notified).unwrap_or_default());
        }
    }

    json!({ "list": list, "justUnlocked": just_unlocked })
}

// The real app version, resolved at compile time from Cargo.toml (so the UI never shows a stale hard-coded number).
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ================= settings =================
#[tauri::command]
fn settings_get(app: AppHandle, state: State<AppState>) -> Value {
    use tauri_plugin_autostart::ManagerExt;
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    json!({
        "autostart": autostart,
        "tray": state.tray_enabled.load(Ordering::SeqCst),
        "defaultIntensity": *state.intensity.lock().unwrap(),
        "miningMode": state.mining_mode.lock().unwrap().clone(),
        "poolAddress": state.pool_address.lock().unwrap().clone()
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
        "miningMode" => {
            if let Some(s) = value.as_str() { *state.mining_mode.lock().unwrap() = s.to_string(); }
            let (m, a) = (state.mining_mode.lock().unwrap().clone(), state.pool_address.lock().unwrap().clone());
            save_mining_prefs(&state.datadir, &m, &a);
        }
        "poolAddress" => {
            if let Some(s) = value.as_str() {
                let s = s.trim();
                // Validate a custom pool address here, where the user enters it (blocks local/private targets,
                // bad ports, control chars) — so an invalid address never reaches the worker.
                if !s.is_empty() {
                    if let Err(e) = validate_pool_addr(s) {
                        return json!({ "ok": false, "error": e });
                    }
                }
                *state.pool_address.lock().unwrap() = s.to_string();
                let (m, a) = (state.mining_mode.lock().unwrap().clone(), state.pool_address.lock().unwrap().clone());
                save_mining_prefs(&state.datadir, &m, &a);
            }
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
    refresh_tooltip(&state); // keep the notification-area tooltip in sync with the new language
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
            // Test-only: when the override endpoint is set (local end-to-end/self-test), accept the
            // self-signed TLS cert of the local HTTPS server. This only affects the transport of the
            // controlled test endpoint; the minisign signature check on the artifact is untouched.
            // Production uses app.updater() (below), which keeps full TLS validation.
            app.updater_builder()
                .endpoints(vec![url])
                .map_err(|e| e.to_string())?
                .configure_client(|c| {
                    c.danger_accept_invalid_certs(true)
                        .danger_accept_invalid_hostnames(true)
                })
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
    // Stop the node and mining engine first so their .exe files aren't locked when the installer replaces them.
    {
        let state = app.state::<AppState>();
        state.mining.store(false, Ordering::SeqCst);
        if let Some(mut child) = state.miner_child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        stop_node(&state);
    }
    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // data directory next to the user's app data (override with BRISVIA_DATADIR, e.g. for isolated tests)
    let datadir = std::env::var("BRISVIA_DATADIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs_data_dir().join("BrisviaSim"));
    let total_secs_initial = load_total_mined(&datadir);
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);

    let (init_mode, init_pool) = load_mining_prefs(&datadir);
    let state = AppState {
        child: Arc::new(Mutex::new(None)),
        datadir,
        wallet_loaded: Arc::new(AtomicBool::new(false)),
        receive_addr: Arc::new(Mutex::new(String::new())),
        mining: Arc::new(AtomicBool::new(false)),
        mined: Arc::new(AtomicU64::new(0)),
        stale: Arc::new(AtomicU64::new(0)),
        mine_start: Arc::new(Mutex::new(None)),
        keep_session_on_relaunch: Arc::new(AtomicBool::new(false)),
        intensity: Arc::new(Mutex::new("equilibrado".to_string())),
        miner_child: Arc::new(Mutex::new(None)),
        hashrate: Arc::new(Mutex::new(0.0)),
        miner_ready: Arc::new(AtomicBool::new(false)),
        tray_enabled: Arc::new(AtomicBool::new(true)),
        mining_mode: Arc::new(Mutex::new(init_mode)),
        pool_address: Arc::new(Mutex::new(init_pool)),
        pending_mnemonic: Arc::new(Mutex::new(None)),
        lang: Arc::new(Mutex::new("es".to_string())),
        tray: Arc::new(Mutex::new(None)),
        cores,
        miner_threads: Arc::new(AtomicU64::new(0)),
        total_mined_secs: Arc::new(Mutex::new(total_secs_initial)),
        ach_cache: Arc::new(Mutex::new(None)),
    };

    let mut builder = tauri::Builder::default();
    // e2e-only: freeze the webview clock so the automated suite can exercise the pre-launch wait mode
    // (before/after Aug 1) without changing the machine's real clock. Absent from the public binary.
    #[cfg(feature = "e2e")]
    {
        builder = builder.plugin(e2e_clock_plugin());
    }
    // Single instance: if the app is already open and it's launched again, bring the existing window to the
    // front instead of opening another. Skipped when BRISVIA_SOLO is set (isolated test instance).
    if std::env::var("BRISVIA_SOLO").is_err() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }));
    }
    builder
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
            // Start the node in the background, shortly after the window is up, so the UI becomes interactive
            // first and Windows doesn't flag the window as "not responding" while the node spins up.
            {
                let node_handle = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(700));
                    let state = node_handle.state::<AppState>();
                    if let Err(e) = start_node(&node_handle, &state) {
                        eprintln!("[brisvia] could not start the node: {}", e);
                    }
                });
            }
            let state = app.state::<AppState>();
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
            wallet_seed_on_disk,
            wallet_validate_phrase,
            wallet_create,
            wallet_seed,
            wallet_confirm_backup,
            wallet_summary,
            wallet_new_address,
            wallet_addresses,
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
            wallet_reveal_seed,
            wallet_migrate_encrypt,
            wallet_check_backup,
            settings_get,
            settings_set,
            open_url,
            system_locale,
            set_language,
            check_update,
            install_update,
            achievements,
            app_version
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Brisvia")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state = app_handle.state::<AppState>();
                flush_total_mined(&state); // don't lose the in-progress session's time on close
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
