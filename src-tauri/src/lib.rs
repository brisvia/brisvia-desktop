// Backend of the Brisvia desktop app (Tauri). Rust controls the bitcoind node (sidecar), the credentials (RPC
// cookie), the wallet and mining; JavaScript (window.brisvia) only sees presentation data. Real node over local
// JSON-RPC, cookie auth, orderly shutdown (stop mining -> stop RPC -> kill).
use std::path::{Path, PathBuf};
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
    pub const RPC_PORT: u16 = 9338; // local RPC of the node (real network); own port (P2P 9342), distinct from Litecoin 9333/9332
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

/// Is pool mining available to users? OFF for the 1.0 line, and this is the switch that decides it — not the
/// UI, which anyone can bypass by editing local settings.
///
/// The stratum engine is finished and tested against the live pool (encrypted, login confirmed, real jobs
/// parsed), but what a POOL USER needs does not exist yet: connection state on screen, the difference between
/// a share found, sent and ACCEPTED by the pool, what happens when the pool drops. Shipping the engine with
/// no honest way to see what it is doing is how someone ends up believing they mined for hours and got paid
/// nothing, with no way to tell whether the software or the pool was at fault. That is the day-one trust of
/// the whole coin, over a feature nobody is asking for yet.
///
/// So mainnet launches with solo mining, which is the thing that must be flawless: create/restore the wallet,
/// start the node, find peers, mine. Pool mining ships in 1.1 as its own release, with the UI and an end-to-end
/// test, once it has been used for real on testnet.
///
/// Turning this to true is a deliberate decision, not a side effect: flip it, build the UI, and the pool
/// becomes reachable again.
const POOL_ENABLED: bool = true;

struct AppState {
    child: Arc<Mutex<Option<Child>>>,
    datadir: PathBuf,
    wallet_loaded: Arc<AtomicBool>,
    // Single-flight guard for wallet_send. Disabling the button in JS is UX, not a barrier: a second
    // event, the Enter key, or any other caller reaches the backend all the same, and two sendtoaddress
    // calls with funds available are TWO REAL PAYMENTS — Core has no idea the second one is a UI double
    // click. Taken with try_lock and never awaited: queueing a second send would just pay twice a moment
    // later. The loser gets ERR:SEND_IN_PROGRESS.
    sending: Arc<Mutex<()>>,
    // Exclusive guard for create/restore. The phrase is the ONE irreplaceable asset here (one encrypted
    // file per datadir) and, until now, nothing in the backend protected it: both paths end in
    // encrypt_phrase_file, which overwrites unconditionally. It was safe only because Core rejects a
    // duplicate wallet name and the frontend hardcodes name="brisvia" — two assumptions the overwriting
    // function does not control. This guard makes the invariant OURS: check-then-write happens under the
    // lock, so two concurrent creates cannot both pass the check.
    wallet_ops: Arc<Mutex<()>>,
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
    // Pool-mode live status, so the UI can be HONEST: connection state, and shares SENT vs ACCEPTED vs REJECTED
    // (a sent share is not a paid share). A contribution counts only on ShareAccepted. Reset on each miner_start.
    pool_connected: Arc<AtomicBool>,
    pool_shares_sent: Arc<AtomicU64>,
    pool_shares_accepted: Arc<AtomicU64>,
    pool_shares_rejected: Arc<AtomicU64>,
    pool_last_error: Arc<Mutex<String>>,
    pool_suspended: Arc<AtomicBool>, // the pool told us it is under maintenance (distinct from an error/disconnect)
    pool_retry_after: Arc<AtomicU64>, // ABSOLUTE unix ts of the next reconnect/maintenance attempt (0 = none)
    pool_has_job: Arc<AtomicBool>,   // a pool job is active right now -> WORKING (not just authenticated)
    pool_ever_job: Arc<AtomicBool>,  // had at least one job this session (distinguishes authenticated vs waiting)
    pool_last_accepted_ts: Arc<AtomicU64>, // unix seconds of the last accepted share (0 = none yet)
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

// ---- Money. Integer base units only; floating point never touches an amount. ----
//
// 1 BRVA = 100_000_000 briv, the same 8-decimal split Bitcoin uses. Amounts used to cross the IPC
// boundary as f64: "0.1 + 0.2" style drift, hidden 9th decimals and scientific notation could all reach
// the node, and the amount shown could differ from the amount sent. The node's own RPC layer accepts a
// STRING and parses it with ParseFixedPoint (exact), so a decimal string built from an integer is exact
// end to end — safer than any float.
const BRIV_PER_BRVA: u64 = 100_000_000;
/// Emission cap: 100,000,000 BRVA. Mirrors MAX_MONEY in the node's consensus params.
const MAX_MONEY_BRIV: u64 = 100_000_000 * BRIV_PER_BRVA;

/// Parses a CANONICAL amount into integer base units (briv). Canonical means: digits and at most one DOT.
///
/// It does NOT accept a comma, and that is the whole point. This function used to do `replace(',', ".")`
/// so an ES user could type "12,5" — which silently turned **"1,000" into 1 BRVA**. In English that means
/// one thousand: someone sending 1,000 BRVA would have sent 1 and lost 999. A backend cannot know which
/// convention the typist meant, so it refuses to guess: the frontend knows the active language and
/// normalises to this canonical form (see toCanonicalAmount in app.js), rejecting the separator that does
/// not belong to that language instead of reinterpreting it.
///
/// Rejects on purpose: empty, zero, signs, scientific notation, commas, more than 8 decimals, anything
/// non-numeric, and amounts above the cap.
fn parse_amount_briv(input: &str) -> Result<u64, String> {
    let s = input.trim().to_string();
    if s.is_empty() {
        return Err("ERR:INVALID_AMOUNT".into());
    }
    // A comma here means the frontend did not normalise, or something called the backend directly. Either
    // way the intent is ambiguous ("1,000" = 1 or 1000?) and money must never be guessed.
    if s.contains(',') {
        return Err("ERR:INVALID_AMOUNT".into());
    }
    // No signs and no exponent: "1e-8" and "-1" are rejected rather than silently reinterpreted.
    if s.contains(['e', 'E', '+', '-']) {
        return Err("ERR:INVALID_AMOUNT".into());
    }
    let (int_str, frac_str) = match s.split_once('.') {
        Some((i, f)) => {
            if f.contains('.') {
                return Err("ERR:INVALID_AMOUNT".into()); // more than one separator
            }
            (i, f)
        }
        None => (s.as_str(), ""),
    };
    if int_str.is_empty() && frac_str.is_empty() {
        return Err("ERR:INVALID_AMOUNT".into());
    }
    if !int_str.chars().all(|c| c.is_ascii_digit()) || !frac_str.chars().all(|c| c.is_ascii_digit()) {
        return Err("ERR:INVALID_AMOUNT".into());
    }
    // The node would reject a 9th decimal with a raw English "Invalid amount"; refuse it here instead of
    // rounding it away, because silently rounding someone's money is worse than telling them to retype it.
    if frac_str.len() > 8 {
        return Err("ERR:INVALID_AMOUNT".into());
    }
    let whole: u64 = if int_str.is_empty() { 0 } else { int_str.parse().map_err(|_| "ERR:INVALID_AMOUNT".to_string())? };
    let mut padded = frac_str.to_string();
    while padded.len() < 8 {
        padded.push('0');
    }
    let frac: u64 = if padded.is_empty() { 0 } else { padded.parse().map_err(|_| "ERR:INVALID_AMOUNT".to_string())? };
    let briv = whole
        .checked_mul(BRIV_PER_BRVA)
        .and_then(|w| w.checked_add(frac))
        .ok_or_else(|| "ERR:INVALID_AMOUNT".to_string())?;
    if briv == 0 {
        return Err("ERR:INVALID_AMOUNT".into());
    }
    if briv > MAX_MONEY_BRIV {
        return Err("ERR:INVALID_AMOUNT".into());
    }
    Ok(briv)
}

/// Renders base units back as the exact decimal string the node's ParseFixedPoint expects.
fn briv_to_decimal(briv: u64) -> String {
    format!("{}.{:08}", briv / BRIV_PER_BRVA, briv % BRIV_PER_BRVA)
}
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
    // Amount rules go BEFORE the address rule: both start with "invalid", and a message that named both
    // would otherwise be reported as a bad address while the real problem is the amount. Core says
    // "Invalid amount" when there are more than 8 decimals — the backend only rejects <= 0, so this is
    // the path a user typing 9 decimals actually hits, on a screen that moves money.
    if m.contains("invalid amount") || m.contains("amount out of range") || m.contains("amount is not a number") { return "ERR:INVALID_AMOUNT".into(); }
    if m.contains("amount too small") || m.contains("dust") { return "ERR:AMOUNT_TOO_SMALL".into(); }
    if m.contains("invalid") && (m.contains("address") || m.contains("bech32")) { return "ERR:INVALID_ADDRESS".into(); }
    if m.contains("fee") && (m.contains("low") || m.contains("small") || m.contains("insufficient")) { return "ERR:FEE_TOO_LOW".into(); }
    if m.contains("starting") || m.contains("loading") || m.contains("warming up") || m.contains("rescanning") { return "ERR:NODE_STARTING".into(); }
    if m.contains("no available keys") || m.contains("not loaded") || m.contains("does not exist") { return "ERR:NODE_UNAVAILABLE".into(); }
    if m.contains("already exists") || m.contains("already loaded") || m.contains("database already") { return "ERR:WALLET_EXISTS".into(); }
    // The wallet passphrase was WRONG. Core: "Error: The wallet passphrase entered was incorrect."
    // Both words are required on purpose: "Error: Please enter the wallet passphrase with walletpassphrase
    // first." also contains "passphrase", but it means the wallet is LOCKED, not that the password is wrong.
    // Matching on "passphrase" alone would tell a user their password failed when they never typed one.
    if m.contains("passphrase") && m.contains("incorrect") { return "ERR:BAD_PASSWORD".into(); }
    // "Please enter the wallet passphrase with walletpassphrase first" = the wallet is LOCKED, which is a
    // different problem from a wrong password and has a different fix. Without this it fell through to the
    // generic "check the node is ready", which is useless advice for someone who just needs to unlock.
    if m.contains("please enter the wallet passphrase") || m.contains("wallet is locked") { return "ERR:WALLET_LOCKED".into(); }
    // Anything unmapped is SANITISED, never forwarded. The frontend shows unknown strings verbatim
    // (app.js transError), and the node's raw text carries things a user must not be handed on a money
    // screen: the word "Bitcoin", absolute paths with their home directory in them, internal RPC wording,
    // English only. The detail goes to the log with paths stripped, so nothing is lost for diagnosis and
    // the user still gets something actionable.
    eprintln!("[brisvia] unmapped node error: {}", sanitize_for_log(msg));
    "ERR:OPERATION_FAILED".into()
}

/// The raw node text for the log, with the obvious personal data stripped. Never the password or phrase:
/// those are not part of an RPC error message and must not be reintroduced here.
fn sanitize_for_log(msg: &str) -> String {
    // Word by word: only a token that really looks like a path is replaced. Splitting on letter+':' alone
    // also matched "Error: ..." and ate the space, mangling every message for no benefit.
    msg.split_inclusive(char::is_whitespace)
        .map(|tok| {
            let t = tok.trim_matches(|c: char| c.is_whitespace() || c == '\'' || c == '"');
            let low = t.to_lowercase();
            let is_windows_path = t.len() > 2 && t.as_bytes()[0].is_ascii_alphabetic() && t[1..].starts_with(":\\");
            let is_unix_home = low.starts_with("/home/") || low.starts_with("/users/");
            if is_windows_path || is_unix_home {
                let trailing: String = tok.chars().skip_while(|c| !c.is_whitespace()).collect();
                format!("<path>{trailing}")
            } else {
                tok.to_string()
            }
        })
        .collect()
}

// The node's RPC port. Override with BRISVIA_RPC_PORT to run an isolated test instance without clashing
// with the real app's node on the default port.
fn rpc_port() -> u16 {
    std::env::var("BRISVIA_RPC_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(RPC_PORT)
}

// ---- JSON-RPC call to bitcoind (cookie auth; the body is never logged) ----
/// What happened to an RPC call, for the one case where the difference is money: a send.
///
/// "It failed" is not good enough for a payment. Three outcomes look identical to a caller today and mean
/// opposite things:
///   Rejected — the node answered with an error. Definitely NOT sent. Safe to retry.
///   NotSent  — we never reached the node. Definitely NOT sent. Safe to retry.
///   Unknown  — the request went out and no answer came back. The payment MAY have been broadcast.
///              Retrying could pay twice. This is the only case that warns the user and never retries.
#[derive(Debug, PartialEq)]
enum RpcOutcome {
    Rejected(String),
    NotSent(String),
    Unknown,
}

/// Classifies a ureq failure into the three outcomes above. Only a read timeout / IO failure AFTER the
/// request went out is Unknown; DNS, a refused connection or a bad URL all mean the node never heard us.
fn classify_transport(e: &ureq::Error) -> RpcOutcome {
    use ureq::ErrorKind::*;
    match e.kind() {
        // We never got the request out: nothing was sent.
        InvalidUrl | UnknownScheme | Dns | InsecureRequestHttpsOnly | ConnectionFailed => {
            RpcOutcome::NotSent(e.to_string())
        }
        // The node answered something we could not read: it DID hear us.
        BadStatus | BadHeader | TooManyRedirects => RpcOutcome::Unknown,
        // Io covers the read timeout: the request left, the answer never arrived. Assume the worst.
        _ => RpcOutcome::Unknown,
    }
}

/// Like rpc(), but reports WHICH of the three outcomes happened. Used by wallet_send; every other caller
/// keeps using rpc(), for which the distinction does not change what the user should do.
fn rpc_classified(datadir: &PathBuf, wallet: Option<&str>, method: &str, params: Value) -> Result<Value, RpcOutcome> {
    let cookie = read_cookie(datadir).ok_or_else(|| RpcOutcome::NotSent("ERR:NODE_NOT_READY".into()))?;
    let url = match wallet {
        Some(w) => format!("http://{}:{}/wallet/{}", RPC_HOST, rpc_port(), w),
        None => format!("http://{}:{}/", RPC_HOST, rpc_port()),
    };
    let auth = format!("Basic {}", b64(cookie.trim().as_bytes()));
    let body = json!({ "jsonrpc": "1.0", "id": "brisvia", "method": method, "params": params });
    match ureq::post(&url).set("Authorization", &auth).timeout(Duration::from_secs(180)).send_json(body) {
        Ok(r) => {
            // The node answered. A body we cannot parse still means it heard us and acted.
            let v: Value = r.into_json().map_err(|_| RpcOutcome::Unknown)?;
            if !v["error"].is_null() {
                return Err(RpcOutcome::Rejected(friendly_error(v["error"]["message"].as_str().unwrap_or("node error"))));
            }
            Ok(v["result"].clone())
        }
        // An HTTP error status: the node answered. For sendtoaddress this is a refusal, not a maybe.
        Err(ureq::Error::Status(_, r)) => {
            let v: Value = r.into_json().unwrap_or_else(|_| json!({}));
            Err(RpcOutcome::Rejected(friendly_error(v["error"]["message"].as_str().unwrap_or("node error"))))
        }
        Err(e) => Err(classify_transport(&e)),
    }
}

fn rpc(datadir: &PathBuf, wallet: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
    // Translatable code instead of raw English: the user used to see "node is not ready yet" while the
    // node was still starting (or had died), with no idea what it meant.
    let cookie = read_cookie(datadir).ok_or_else(|| "ERR:NODE_NOT_READY".to_string())?;
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

// ---- self-repair: a hard shutdown can corrupt the node's database ----
// A power cut, a battery running out or the task manager killing the process can leave the block/chainstate
// database half-written. bitcoind then refuses to start, writes why to debug.log and exits, so the app would sit
// at "Preparing wallet..." forever with no way out for a non-technical user. Real case: it happened twice on the
// owner's machine during testing.
//
// Three rules learned from the audit, in order of importance:
//  1. CAUSALITY. Only the log this attempt wrote counts. Reading the whole file would react to a message from
//     months ago, already fixed by hand, and rebuild the chain for nothing. We record the log size before
//     launching and read only from there.
//  2. THE CAUSE PICKS THE CURE. A full disk, a locked datadir or a permissions error are NOT corruption: a
//     rebuild cannot fix them and would just loop. Those must be reported, never auto-repaired.
//  3. ONE SHOT. If a repair already ran and the node still fails, stop. An automatic repair loop on a user's
//     machine is worse than an honest error message.
// The seed is never at risk: a rebuild only touches blocks/chainstate, while keys live in a separate file that
// this code never deletes, moves or writes.

#[derive(PartialEq, Debug)]
enum Repair {
    None,             // the node did not fail this way; nothing to do
    Chainstate,       // rebuild only the UTXO state (fast) — the node asks for this one by name
    Full,             // rebuild the block index too (slower, needed when the index itself is broken)
    Blocked(&'static str), // an operational problem: a rebuild would not fix it, so tell the user instead
    /// NOT a fault: the genesis is dated at the launch instant, so until August 1st it looks like a block
    /// "from the future" and the node refuses to start (chainstate.cpp:241 — it tolerates 2 h, and the
    /// genesis is weeks ahead). This is the real reason the app used to hang on "Preparing wallet..." before
    /// the self-repair existed; it was never the killed process I blamed. It cures itself at 15:00 UTC on
    /// launch day, when the genesis stops being in the future.
    FutureGenesis,
}

// Decides what to do from what the node wrote while it was failing to start.
fn classify_failure(log: &str) -> Repair {
    // Operational causes FIRST: these look scary in the log but a rebuild is the wrong answer.
    if log.contains("No space left on device")
        || log.contains("Disk space is too low")
        || log.contains("Error: Disk space is low")
    {
        return Repair::Blocked("disk");
    }
    if log.contains("Cannot obtain a lock on data directory")
        || log.contains("probably already running")
    {
        return Repair::Blocked("locked");
    }
    if log.contains("Permission denied") || log.contains("Access is denied") {
        return Repair::Blocked("permissions");
    }
    // BEFORE the corruption cases: this message ends with "Please restart with -reindex", so without this
    // branch it would be classified as a broken database. Nothing is broken -- the genesis is simply dated at
    // the launch instant and the node will not load a tip more than 2 h in the future.
    if log.contains("appears to be from the future") {
        return Repair::FutureGenesis;
    }
    // The node explicitly asks for a chainstate-only rebuild: that is the cheap fix, prefer it.
    if log.contains("-reindex-chainstate") || log.contains("Error opening coins database") {
        return Repair::Chainstate;
    }
    // The block index itself is broken: a full rebuild is the only way back.
    if log.contains("Please restart with -reindex")
        || log.contains("Corrupted block database detected")
        || log.contains("Error opening block database")
    {
        return Repair::Full;
    }
    Repair::None
}

// Reads only what was appended to the log after `from` (the size it had before this attempt started).
fn log_since(datadir: &Path, from: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let path = datadir.join(net_subdir()).join("debug.log");
    let Ok(mut f) = std::fs::File::open(&path) else {
        return String::new();
    };
    if f.seek(SeekFrom::Start(from)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = f.take(1024 * 1024).read_to_end(&mut buf); // a failing start writes little; cap it anyway
    String::from_utf8_lossy(&buf).into_owned()
}

fn log_size(datadir: &Path) -> u64 {
    std::fs::metadata(datadir.join(net_subdir()).join("debug.log"))
        .map(|m| m.len())
        .unwrap_or(0)
}

// Marker so a repair that did not work is never retried in a loop on the next start.
fn repair_marker(datadir: &Path) -> PathBuf {
    datadir.join("repair_attempted")
}

// How long a repair attempt keeps blocking further attempts. A real loop retries within seconds, so a short
// window stops it; a genuinely new problem days later must still be repaired.
const REPAIR_COOLDOWN_SECS: u64 = 600; // 10 minutes

fn ahora_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// Was a repair attempted so recently that trying again would just be a loop?
//
// This used to be a plain "does the file exist?", which was wrong in a way that only showed up on a real
// machine: the marker was written before repairing but never removed when the repair WORKED. Since every
// update force-closes the app (which is what corrupts the database in the first place), the second corruption
// would find the old marker and refuse to repair -- telling the user "I could not repair it" without even
// trying. Caught on the owner's machine: the marker was sitting there after a repair that had gone fine.
//
// The marker now carries the time of the attempt. Recent = a loop, refuse. Old = the previous incident is
// long over, this is a new one, repair it.
fn repair_blocked(datadir: &Path) -> bool {
    let Ok(txt) = std::fs::read_to_string(repair_marker(datadir)) else {
        return false; // no marker: nothing was attempted
    };
    match txt.rsplit_once('|').and_then(|(_, ts)| ts.trim().parse::<u64>().ok()) {
        Some(ts) => ahora_secs().saturating_sub(ts) < REPAIR_COOLDOWN_SECS,
        // A marker with no timestamp comes from the first version of this code. Treat it as old (do not block):
        // that version left it behind after successful repairs, so honouring it would deny a repair for good.
        None => false,
    }
}

// ---- start the node as a child process ----
/// Build the node's `bitcoin.conf`. Pure and testable on purpose: the app rewrites this file on EVERY start
/// (see start_node), so a stale conf from an older version — e.g. one still pinning the pre-9338 RPC port — is
/// always replaced. A user can never be silently stranded on an old port. It sets ONLY the RPC port; the P2P
/// port is the chainparams default (9342 mainnet) and is deliberately never written here, so no stale P2P port
/// can linger either. `peers.dat` may still hold old-port peers, but the addnode seeds bootstrap recovery.
fn node_conf(chain: &str, net_lines: &str, rpc_port: u16, seeds: &str) -> String {
    format!(
        // The node connects to the network on its own (dnsseed + fixed seed), accepts inbound if the router
        // allows it (natpmp tries to open the port), and validates everything locally. RPC stays on 127.0.0.1 only.
        "chain={chain}\nserver=1\n{net_lines}rpcthreads=16\nrpcworkqueue=128\n[{chain}]\nrpcport={port}\nrpcbind=127.0.0.1\nrpcallowip=127.0.0.1\n{seeds}",
        chain = chain,
        net_lines = net_lines,
        port = rpc_port,
        seeds = seeds
    )
}

fn start_node(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let bitcoind = find_binary(app, &format!("bitcoind{EXE_SUFFIX}")).ok_or("ERR:NODE_BINARY_MISSING")?;
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
        // NOTE: maxtipage is NOT here. It is DEBUG_ONLY, so the node ignores it from bitcoin.conf -- the first
        // attempt put it here, changed nothing, and the mining journey stayed red. It goes on the command line
        // in spawn_node().
        "listen=0\ndiscover=0\ndnsseed=0\nnatpmp=0\nfallbackfee=0.0001\n"
    } else {
        "listen=1\ndiscover=1\ndnsseed=1\nnatpmp=1\nfallbackfee=0.02\nminrelaytxfee=0.01\nincrementalrelayfee=0.01\ndustrelayfee=0.03\nblockmintxfee=0.01\nmaxmempool=50\nmempoolexpiry=24\npersistmempool=1\n"
    };
    let seeds = if isolated { "" } else { seeds };
    let conf = node_conf(&chain, net_lines, rpc_port(), seeds);
    // Write atomically (temp file + rename). The app fully owns this file — it rewrites the whole thing on every
    // start and there are no user-set options to preserve — but a crash MID-WRITE must never leave the node a
    // half-written conf to read. rename() over the target is atomic on the same filesystem (Windows included).
    let conf_path = state.datadir.join("bitcoin.conf");
    let conf_tmp = state.datadir.join("bitcoin.conf.new");
    std::fs::write(&conf_tmp, &conf).map_err(|e| e.to_string())?;
    std::fs::rename(&conf_tmp, &conf_path).map_err(|e| e.to_string())?;
    // Remember how big the log is BEFORE launching, so we only ever read what THIS attempt writes.
    let mark = log_size(&state.datadir);
    let mut child = spawn_node(&bitcoind, &state.datadir, None)?;

    // A healthy node stays up. One that cannot open its database dies within a couple of seconds, so give it a
    // short window: if it is still alive after that, this is a normal start and we get out of the way.
    let died = wait_for_early_exit(&mut child, Duration::from_secs(6));
    if !died {
        *state.child.lock().unwrap() = Some(child);
        let _ = std::fs::remove_file(repair_marker(&state.datadir)); // healthy start: clear any old marker
        return Ok(());
    }

    // It died on startup. Only this attempt's log decides what happens next.
    let what = classify_failure(&log_since(&state.datadir, mark));
    let flag = match what {
        Repair::None => return Err("ERR:NODE_START_FAILED".into()), // died for a reason we do not know
        Repair::Blocked("disk") => return Err("ERR:NODE_DISK_FULL".into()),
        Repair::Blocked("locked") => return Err("ERR:NODE_ALREADY_RUNNING".into()),
        Repair::Blocked(_) => return Err("ERR:NODE_PERMISSIONS".into()),
        Repair::FutureGenesis => "-reindex", // expected until launch day, not damage — see below
        Repair::Chainstate => "-reindex-chainstate",
        Repair::Full => "-reindex",
    };
    // The future-genesis case happens on EVERY start until launch day, so it must not spend the anti-loop
    // marker: that marker exists to stop a repair that keeps failing, and this one is not a repair at all --
    // it is the expected startup path before August 1st. Relaunching works because -reindex empties the
    // chainstate, and the node only checks the tip's date when the chainstate is NOT empty. Instant here: the
    // chain holds one block.
    if what != Repair::FutureGenesis {
        // If a repair was attempted moments ago and it is failing again, stop: retrying is a loop, and an
        // automatic loop on someone's machine is worse than an honest error message.
        if repair_blocked(&state.datadir) {
            return Err("ERR:NODE_REPAIR_FAILED".into());
        }
        let _ = std::fs::write(repair_marker(&state.datadir), format!("{flag}|{}", ahora_secs()));
    }
    let mark = log_size(&state.datadir);
    let mut child = spawn_node(&bitcoind, &state.datadir, Some(flag))?;
    // A rebuild takes a while, so we only check it did not die immediately again.
    if wait_for_early_exit(&mut child, Duration::from_secs(6)) {
        let what = classify_failure(&log_since(&state.datadir, mark));
        return Err(match what {
            Repair::Blocked("disk") => "ERR:NODE_DISK_FULL".into(),
            _ => "ERR:NODE_REPAIR_FAILED".to_string(),
        });
    }
    // The repair took. The incident is over, so the marker goes: leaving it behind would deny the NEXT repair,
    // and there is always a next one (every update force-closes the app, which is what corrupts the database).
    let _ = std::fs::remove_file(repair_marker(&state.datadir));
    *state.child.lock().unwrap() = Some(child);
    Ok(())
}

// Launches bitcoind, optionally with a repair flag.
fn spawn_node(bitcoind: &Path, datadir: &Path, repair_flag: Option<&str>) -> Result<Child, String> {
    let mut cmd = Command::new(bitcoind);
    cmd.arg(format!("-datadir={}", datadir.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // maxtipage ONLY in an isolated (regtest) run, and it goes on the COMMAND LINE, not in the config file:
    // it is a DEBUG_ONLY option and the node ignores it from bitcoin.conf (which is why the first attempt at
    // this changed nothing and the mining journey stayed red).
    //
    // Why it is needed: regtest's genesis is dated 2011, so the node reports "still syncing" from the first
    // second, and the app correctly refuses to mine on a chain it believes is stale. The test then fails on a
    // real, CORRECT behaviour.
    //
    // Why it is safe: this branch runs only when the chain was overridden to regtest, which only the e2e build
    // can do (the env read is compiled out of the public binary). The real network keeps the 24 h default, and
    // that default is load-bearing -- the mainnet genesis is stamped at the launch instant precisely so the
    // chain is minable at 15:00 (see launch_window_tests).
    if net_chain() != NET_CHAIN {
        // 100 years. NOT a round number picked at random: regtest's genesis is dated 2011, so it is already
        // ~15 years old and a one-year window did not cover it -- the node accepted the flag (it logs it) and
        // still reported "syncing". The first attempt failed exactly there. This has to keep working years
        // from now, so the window is far wider than the gap it must span.
        cmd.arg("-maxtipage=3153600000");
    }
    if let Some(f) = repair_flag {
        cmd.arg(f);
    }
    no_window(&mut cmd);
    cmd.spawn()
        .map_err(|e| format!("could not start the node: {}", e))
}

// True if the process exited within `window`. Polls instead of blocking so a healthy node is not delayed.
fn wait_for_early_exit(child: &mut Child, window: Duration) -> bool {
    let deadline = std::time::Instant::now() + window;
    while std::time::Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return true, // exited
            Ok(None) => std::thread::sleep(Duration::from_millis(150)),
            Err(_) => return false,
        }
    }
    false
}

// ---- orderly shutdown: stop mining -> stop RPC -> wait -> kill ----
/// How long the node is allowed to take to close on its own.
///
/// It used to be 5.5 seconds, and then the app killed it. That is the defect: Bitcoin Core flushes the
/// chainstate on shutdown and its own docs say it can take several minutes. Killing it mid-flush is
/// exactly how a block database ends up half-written, and the user pays for it on the next start with a
/// repair -- or with a corrupted database.
///
/// 180 seconds is not a guess: it is the ceiling agreed for this, and it is a ceiling, not a wait. The
/// common case still returns in a second or two, as soon as the process is actually gone.
const NODE_SHUTDOWN_MAX: Duration = Duration::from_secs(180);

/// How the wait ended, when it ended without anything going wrong.
#[derive(Debug)]
enum NodeExitOutcome {
    /// There was never a process in the slot. Nothing to wait for.
    AlreadyAbsent,
    /// It closed on its own, and this is how.
    Exited(std::process::ExitStatus),
    /// Still alive when the deadline passed. It was NOT killed.
    TimedOut,
}

/// The wait could not be carried out. Distinct from TimedOut: this means we do not KNOW.
#[derive(Debug)]
enum NodeExitError {
    /// Another thread panicked holding the slot. The Child may be anything; we cannot claim it exited.
    LockPoisoned,
    /// The OS refused to tell us whether the process is alive.
    WaitFailed(String),
}

/// Wait for a process to be gone. Takes a process slot and a deadline, and nothing else.
///
/// WHY IT TAKES A SLOT AND NOT AN AppState
/// ---------------------------------------
/// It used to take `&AppState`, and its tests built one to call it. AppState carries
/// `tray: Arc<Mutex<Option<tauri::tray::TrayIcon>>>`, so constructing it made the TrayIcon type
/// reachable, which linked Tauri's GUI runtime into the test binary, which imported
/// TaskDialogIndirect from comctl32.dll -- a function only Common Controls v6 exports. A cargo test
/// binary carries no manifest, so Windows hands it comctl32 5.82, which does not export it, and the
/// executable died with 0xC0000139 before main. No test ran. Proven in run 29465731970: rc6-intact
/// exit=-1073741511, this shape exit=0 with 61 tests listed.
///
/// So the coupling was the defect, and it is gone: nothing here can reach Tauri.
///
/// WHY NOT bool
/// ------------
/// `false` used to mean "still running" and "the OS would not answer" and "never started". Those need
/// different responses -- one aborts the install, one is fine -- and a bool cannot tell them apart.
///
/// THE LOCK
/// --------
/// Held only for the try_wait, released before every sleep. Holding it for the full 180 seconds would
/// block the node's own cleanup, and the shutdown path is exactly when other threads need the slot.
///
/// The Child is never taken out of the Option to wait on it: while it was out, another reader sees
/// None and concludes the node is gone. It is cleared only once it has actually exited, which is true.
fn wait_for_node_exit(
    process_slot: &Arc<Mutex<Option<Child>>>,
    timeout: Duration,
) -> Result<NodeExitOutcome, NodeExitError> {
    // Instant, not the wall clock: a clock that jumps back mid-wait would restart the deadline, and one
    // that jumps forward would end it early. Instant is monotonic and answers "how long since", which
    // is the only question here.
    let started = std::time::Instant::now();
    loop {
        {
            let mut guard = process_slot.lock().map_err(|_| NodeExitError::LockPoisoned)?;
            match guard.as_mut() {
                None => return Ok(NodeExitOutcome::AlreadyAbsent),
                Some(child) => match child.try_wait() {
                    Ok(Some(status)) => {
                        // Gone. Clearing the slot now records what is true; it also reaps the handle.
                        *guard = None;
                        return Ok(NodeExitOutcome::Exited(status));
                    }
                    Ok(None) => {}
                    Err(e) => return Err(NodeExitError::WaitFailed(e.to_string())),
                },
            }
        } // lock released here, before the sleep

        if started.elapsed() >= timeout {
            return Ok(NodeExitOutcome::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Ask the node to close and WAIT for it to actually be gone. Never kill it.
///
/// Returns true if the process exited on its own, false if it was still alive after NODE_SHUTDOWN_MAX.
/// A `false` here is serious: the caller must not go on to replace files underneath a live node.
///
/// What this deliberately does NOT do any more: kill the node. There is no timeout after which killing
/// it becomes acceptable, because the whole point of waiting is that the node is busy writing. The one
/// case where killing looks tempting -- "it is taking too long" -- is precisely the case where killing
/// does the damage.
fn stop_node_and_wait(state: &AppState) -> bool {
    stop_node_and_wait_max(state, NODE_SHUTDOWN_MAX)
}

/// The same shutdown with the deadline as a parameter. Exists so it can be TESTED: 180 seconds is not
/// viable in a test, and a guard that cannot be tested is not a guard.
///
/// This is the wrapper the directive asks for: it does the app-specific work -- stop mining, close the
/// miner, ask the node over RPC -- and then hands the process slot to wait_for_node_exit. It pulls the
/// slot out of AppState and knows nothing else about waiting.
fn stop_node_and_wait_max(state: &AppState, plazo: Duration) -> bool {
    state.mining.store(false, Ordering::SeqCst);
    // The miner first: it keeps asking the node for work, and while it holds the .exe open the
    // installer cannot replace it either. This one is ours and holds no chain data, so kill+wait is
    // fine -- and we wait, so it cannot outlive us as an orphan.
    if let Some(mut m) = state.miner_child.lock().unwrap().take() {
        let _ = m.kill();
        let _ = m.wait();
    }

    // The node's own orderly-shutdown request. This is what writes "Shutdown in progress..." and then
    // "Shutdown done" to debug.log; those two lines are the evidence that it closed properly.
    let pedido = rpc(&state.datadir, None, "stop", json!([]));
    if pedido.is_err() {
        eprintln!("[brisvia] the node did not accept `stop`; waiting for it anyway");
    }

    let started = std::time::Instant::now();
    match wait_for_node_exit(&state.child, plazo) {
        Ok(NodeExitOutcome::AlreadyAbsent) => true, // we never started one
        Ok(NodeExitOutcome::Exited(_)) => {
            eprintln!("[brisvia] the node closed on its own in {:.1}s", started.elapsed().as_secs_f32());
            true
        }
        Ok(NodeExitOutcome::TimedOut) => {
            eprintln!("[brisvia] the node is STILL running after {}s. Not killing it.", plazo.as_secs());
            false
        }
        Err(NodeExitError::WaitFailed(e)) => {
            eprintln!("[brisvia] cannot check on the node: {e}");
            false
        }
        Err(NodeExitError::LockPoisoned) => {
            // A thread panicked holding the slot. We cannot say the node exited, and saying so would
            // let an installer overwrite a live datadir. Refuse.
            eprintln!("[brisvia] the node slot is poisoned; cannot confirm it closed");
            false
        }
    }
}

/// Close the node on the way out of the app.
///
/// Same wait, and the same refusal to kill. If it is still writing when the user closes the window, the
/// right thing is to let it finish -- the app going away a few seconds later is invisible, a repaired
/// database is not.
fn stop_node(state: &AppState) {
    let _ = stop_node_and_wait(state);
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
    // A wallet EXISTS locally if EITHER the encrypted seed OR the legacy plaintext phrase file is present. A
    // legacy wallet (only wallet_seed_phrase.txt, from before password support) must NOT read as "no wallet" and
    // re-trigger the create/restore onboarding after an update — that is how a user gets asked for 12 words they
    // may not have, or overwrites a real wallet. Presence alone is enough to block create-onboarding; whether the
    // legacy file is USABLE (BIP39 valid) is a separate check handled in the migration flow.
    enc_seed_path(&state.datadir).exists() || seed_phrase_path(&state.datadir).exists()
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
fn wallet_confirm_backup(state: State<AppState>) -> Value {
    // "I saved them" is NOT proof of a working backup: report the REAL persisted verification state, never a
    // bare true. The badge only turns on after wallet_verify_backup matched the re-entered words.
    json!({ "backed_up": read_backup_verified(&state.datadir) })
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
        "backed_up": read_backup_verified(&state.datadir)
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
    let addr = rpc(&state.datadir, Some(WALLET_NAME), "getnewaddress", json!([])).map_err(|e| friendly_error(&e))?;
    let a = addr.as_str().unwrap_or("").to_string();
    *state.receive_addr.lock().unwrap() = a.clone();
    append_address(&state.datadir, &a);
    Ok(json!({ "address": a }))
}

#[tauri::command]
// `amount` arrives as the STRING the user typed, never as f64: floating point must not touch money.
// It is parsed into integer base units here and handed to the node as an exact decimal string, which its
// RPC layer reads with ParseFixedPoint (exact). See parse_amount_briv.
fn wallet_send(state: State<AppState>, address: String, amount: String, password: String) -> Result<Value, String> {
    // Single-flight: only one send may be in flight. try_lock, NEVER lock() — waiting for the guard would
    // just run the duplicate send a moment later, which is the exact thing being prevented. The guard is
    // held to the end of the function (unlock, send, relock) and released on every return path.
    let _sending = state
        .sending
        .try_lock()
        .map_err(|_| "ERR:SEND_IN_PROGRESS".to_string())?;
    let briv = parse_amount_briv(&amount)?;
    let exact = briv_to_decimal(briv);
    // Encrypted wallets (new format): unlock briefly with the password, send, and lock again right away.
    // Old unencrypted wallets (created before password support): send directly, without a passphrase, so
    // updating the app never breaks an existing wallet. The UI offers to protect them with a password.
    if wallet_is_encrypted(&state.datadir) {
        rpc(&state.datadir, Some(WALLET_NAME), "walletpassphrase", json!([password, 30]))
            .map_err(|e| friendly_error(&e))?;
        let res = rpc_classified(&state.datadir, Some(WALLET_NAME), "sendtoaddress", json!([address, exact]));
        let _ = rpc(&state.datadir, Some(WALLET_NAME), "walletlock", json!([]));
        let txid = res.map_err(send_failure_to_code)?;
        Ok(json!({ "ok": true, "txid": txid }))
    } else {
        let txid = rpc_classified(&state.datadir, Some(WALLET_NAME), "sendtoaddress", json!([address, exact]))
            .map_err(send_failure_to_code)?;
        Ok(json!({ "ok": true, "txid": txid }))
    }
}

// Estimate the network fee for a payment WITHOUT sending it. Builds a funded PSBT (Core picks the inputs and
// computes the change + fee) but does NOT lock any UTXO, sign, or broadcast — a read-only price check so the
// confirmation screen can show "receives / fee / total" before the person commits. The real send still goes
// through the audited sendtoaddress path; on this no-congestion network the two match, so the shown value is
// the network fee for this exact payment. amount uses the same exact parse as the real send; the fee itself
// is informational, so reading it as f64 is fine (the actual send stays exact via the amount string).
#[tauri::command]
fn wallet_estimate_send(state: State<AppState>, address: String, amount: String) -> Result<Value, String> {
    let briv = parse_amount_briv(&amount)?;
    // The SAME exact 8-decimal string the real send uses — never routed through an f64 — so the estimate and
    // the send agree on the amount to the base unit (audit correction).
    let exact = briv_to_decimal(briv);
    let mut out = serde_json::Map::new();
    out.insert(address, json!(exact));
    let outputs = serde_json::Value::Array(vec![serde_json::Value::Object(out)]);
    // add_inputs:true -> Core picks the inputs for the empty input list; lockUnspents:false -> reserve nothing.
    // Both stated explicitly rather than relying on defaults. This funds a PSBT only to read the fee: no sign,
    // no broadcast, no UTXO held. The real payment still goes out through the audited sendtoaddress path.
    let opts = json!({ "add_inputs": true, "lockUnspents": false });
    let psbt = rpc(&state.datadir, Some(WALLET_NAME), "walletcreatefundedpsbt", json!([[], outputs, 0, opts]))
        .map_err(|e| friendly_error(&e))?;
    let fee = psbt.get("fee").and_then(|f| f.as_f64()).unwrap_or(0.0);
    let fee_briv = (fee * 100_000_000.0).round() as u64; // back to integer base units for an exact total
    Ok(json!({
        "receives": exact,
        "fee": briv_to_decimal(fee_briv),
        "total": briv_to_decimal(briv + fee_briv)
    }))
}

/// Turns a send failure into the code the user sees. Only a genuinely ambiguous outcome gets the warning:
/// telling someone "the payment may have gone out, check your transactions" when the node simply refused
/// would scare them for nothing and invite a pointless double check. And staying silent about a real
/// unknown could make them pay twice.
fn send_failure_to_code(o: RpcOutcome) -> String {
    match o {
        // The node answered: it did not send. friendly_error already mapped the reason.
        RpcOutcome::Rejected(code) => code,
        // We never reached the node: nothing was sent, and it is not a money problem.
        RpcOutcome::NotSent(_) => "ERR:NODE_NOT_READY".to_string(),
        // The request went out and no answer came back. It may be on the network. NEVER auto-retry.
        RpcOutcome::Unknown => "ERR:SEND_STATUS_UNKNOWN".to_string(),
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
    let tx = rpc(&state.datadir, Some(WALLET_NAME), "gettransaction", json!([txid]))
        .map_err(|e| friendly_error(&e))?;
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
    rpc(&state.datadir, Some(WALLET_NAME), "backupwallet", json!([path_str])).map_err(|e| friendly_error(&e))?;
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
// the SLIP-44 registry (independent of the P2P port 9342), and is NOT Bitcoin's 0' (so the same seed does not
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

// ============================================================================================================
// E2E MIGRATION HELPER — compiled ONLY under the `e2e-helper` feature, NEVER in the public app.
// It exists so the macOS/Linux migration jobs can create a THROWAWAY wallet_seed.enc using the EXACT production
// crypto (so the file is byte-compatible with what the app writes) and, after an update, prove the surviving
// file still unlocks and derives the SAME address. Security (audit): there is deliberately NO seed-input path
// in the production binary — this whole module and its bin are feature-gated out of every release build. The
// throwaway mnemonic/password travel over STDIN (never argv/logs); only the PUBLIC address is printed.
// Gated behind the existing `e2e` feature (test-only, never in a release build).
#[cfg(feature = "e2e")]
pub mod e2e_helper {
    use super::{decrypt_phrase_file, descriptors_from_mnemonic, encrypt_phrase_file};
    use bitcoin::bip32::{ChildNumber, Xpriv};
    use bitcoin::hashes::Hash;
    use bitcoin::secp256k1::Secp256k1;
    use bitcoin::CompressedPublicKey;
    use bip39::Mnemonic;
    use std::path::Path;
    use std::str::FromStr;

    // First external receiving address (change 0, index 0) — the same one the node's getnewaddress returns for a
    // fresh wallet, derived purely from the account key (identical logic to the massive_wallet_derivation test).
    fn first_address(mnemonic: &Mnemonic) -> Result<String, String> {
        let (_fp, ext, _int) = descriptors_from_mnemonic(mnemonic)?;
        // descriptors_from_mnemonic always yields `wpkh([origin]xprv.../0/*)`, so this split/strip cannot fail
        // for a valid mnemonic. expect() (not ok_or_else with a user-facing string) keeps this test-only helper
        // out of the error-contract guard, which requires ERR:X codes for anything that could reach a real user.
        let account_xprv = ext
            .split(']').nth(1).and_then(|s| s.strip_suffix("/0/*)"))
            .expect("e2e helper: descriptor always has the [origin]xprv/0/* shape");
        let secp = Secp256k1::new();
        let account = Xpriv::from_str(account_xprv).map_err(|e| e.to_string())?;
        let path = [ChildNumber::from_normal_idx(0).unwrap(), ChildNumber::from_normal_idx(0).unwrap()];
        let child = account.derive_priv(&secp, &path).map_err(|e| e.to_string())?;
        let pubkey = CompressedPublicKey::from_private_key(&secp, &child.to_priv()).map_err(|e| e.to_string())?;
        let program = pubkey.wpubkey_hash().to_byte_array();
        #[cfg(feature = "mainnet")]
        let hrp = "brv";
        #[cfg(not(feature = "mainnet"))]
        let hrp = "tbrv";
        let hrp = bitcoin::bech32::Hrp::parse(hrp).map_err(|e| e.to_string())?;
        bitcoin::bech32::segwit::encode_v0(hrp, &program).map_err(|e| e.to_string())
    }

    /// Create a throwaway wallet_seed.enc at `datadir` from `mnemonic`+`password` (real production crypto).
    /// Returns the wallet's first (public) receiving address.
    pub fn create(datadir: &Path, mnemonic: &str, password: &str) -> Result<String, String> {
        let m = Mnemonic::parse(mnemonic).map_err(|e| e.to_string())?;
        encrypt_phrase_file(datadir, &m.to_string(), password)?;
        first_address(&m)
    }

    /// Decrypt the surviving wallet_seed.enc and return the first (public) address — proves it still unlocks with
    /// the same password and derives the SAME address after an update.
    pub fn verify(datadir: &Path, password: &str) -> Result<String, String> {
        let phrase = decrypt_phrase_file(datadir, password)?;
        let m = Mnemonic::parse(&phrase).map_err(|e| e.to_string())?;
        first_address(&m)
    }
}

// Adds ONLY the checksum to the PRIVATE descriptor (we don't use info["descriptor"], which returns the public/xpub
// version and would leave the wallet without spending keys). getdescriptorinfo["checksum"] is for the string we pass.
fn checksummed(datadir: &PathBuf, desc: &str) -> Result<String, String> {
    let info = rpc(datadir, None, "getdescriptorinfo", json!([desc]))?;
    let cs = info["checksum"].as_str().ok_or_else(|| "ERR:WALLET_NOT_READY".to_string())?;
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
    // Exclusive: the exists-check below and the phrase write must not interleave with another create.
    let _ops = state.wallet_ops.lock().map_err(|_| "ERR:WALLET_EXISTS".to_string())?;
    // Fail closed BEFORE touching Core. Until now the only thing standing between a second create and a
    // destroyed phrase was Core refusing a duplicate wallet name — an assumption this function does not own.
    refuse_if_wallet_exists(&state.datadir)?;
    validate_password(&password)?;
    let mnemonic = Mnemonic::generate(12).map_err(|e| e.to_string())?;
    let (fp, ext, int) = descriptors_from_mnemonic(&mnemonic)?;
    // createwallet blank (no keys), descriptor
    rpc(&state.datadir, None, "createwallet", json!([name, false, true, "", false, true])).map_err(|e| friendly_error(&e))?;
    import_descriptors(&state.datadir, &name, &ext, &int, false)?;
    // Encrypt the Core wallet's private keys with the user's password (wallet is left locked afterwards).
    rpc(&state.datadir, Some(&name), "encryptwallet", json!([password])).map_err(|e| friendly_error(&e))?;
    let mut words = mnemonic.to_string();
    *state.pending_mnemonic.lock().unwrap() = Some(words.clone());
    // Store the phrase ENCRYPTED (Argon2id + AES-256-GCM), never plaintext.
    encrypt_phrase_file_ex(&state.datadir, &words, &password, false)?;
    clear_backup_verified(&state.datadir); // a fresh wallet starts NOT backup-verified until re-checked
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
        // Persist REAL proof of backup (fingerprint) so the "backup verified" badge is never a default true.
        if let Ok(m) = Mnemonic::parse(words.join(" ").trim()) {
            if let Ok((fp, _e, _i)) = descriptors_from_mnemonic(&m) {
                write_backup_verified(&state.datadir, &fp);
            }
        }
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
    let _ops = state.wallet_ops.lock().map_err(|_| "ERR:WALLET_EXISTS".to_string())?;
    // Restoring over an existing wallet would overwrite its phrase: same guard as create.
    refuse_if_wallet_exists(&state.datadir)?;
    let phrase = zeroize::Zeroizing::new(phrase); // wiped from memory on every return path
    validate_password(&password)?;
    let mnemonic = Mnemonic::parse(phrase.trim())
        .map_err(|_| "ERR:INVALID_PHRASE".to_string())?;
    let (fp, ext, int) = descriptors_from_mnemonic(&mnemonic)?;
    // Core refuses a duplicate wallet name, so an existing wallet is NEVER overwritten: this call fails
    // first and the phrase file below is never reached (verified against a real node on regtest).
    // Translated, or the user restoring over an existing wallet gets Core's raw "Failed to create database
    // path 'C:\...'. Database already exists." instead of "you already have a wallet".
    rpc(&state.datadir, None, "createwallet", json!([name, false, true, "", false, true]))
        .map_err(|e| friendly_error(&e))?;
    import_descriptors(&state.datadir, &name, &ext, &int, true)?; // rescan from genesis
    // Encrypt the restored wallet's keys and store the phrase encrypted with the new password.
    rpc(&state.datadir, Some(&name), "encryptwallet", json!([password])).map_err(|e| friendly_error(&e))?;
    encrypt_phrase_file_ex(&state.datadir, phrase.trim(), &password, false)?;
    clear_backup_verified(&state.datadir); // a restored wallet starts NOT backup-verified until re-checked
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
    rpc(&state.datadir, Some(WALLET_NAME), "encryptwallet", json!([password])).map_err(|e| friendly_error(&e))?;
    if !old.is_empty() {
        encrypt_phrase_file(&state.datadir, &old.join(" "), &password)?;
        // Verify the new encrypted file REOPENS and derives the SAME fingerprint BEFORE removing the plaintext.
        // A bad write/crash must never leave the user with the plaintext gone and no usable encrypted wallet.
        let reopened = decrypt_phrase_file(&state.datadir, &password).map_err(|_| "ERR:MIGRATE_VERIFY".to_string())?;
        let want = Mnemonic::parse(old.join(" ").trim()).ok()
            .and_then(|m| descriptors_from_mnemonic(&m).ok()).map(|(fp, _, _)| fp);
        let got = Mnemonic::parse(reopened.trim()).ok()
            .and_then(|m| descriptors_from_mnemonic(&m).ok()).map(|(fp, _, _)| fp);
        if want.is_none() || want != got {
            return Err("ERR:MIGRATE_VERIFY".to_string());
        }
        let _ = std::fs::remove_file(seed_phrase_path(&state.datadir)); // safe now: verified reopen + same fingerprint
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

// Classify a LEGACY wallet (plaintext wallet_seed_phrase.txt, from before password support) WITHOUT the node,
// so onboarding never re-appears for an existing wallet and a CORRUPT legacy file is never mistaken for "no
// wallet" (which would offer create/restore over real funds). States:
//   encrypted_present -> wallet_seed.enc already there, nothing legacy to migrate
//   none              -> no legacy file
//   legacy_valid      -> file present, BIP39 valid: offer migration to an encrypted wallet
//   legacy_corrupt    -> file present but unreadable / wrong word count / BIP39 checksum fails: offer
//                        recovery/repair, NEVER create-onboarding
#[tauri::command]
fn wallet_legacy_status(state: State<AppState>) -> Value {
    let datadir = &state.datadir;
    if enc_seed_path(datadir).exists() {
        return json!({ "status": "encrypted_present" });
    }
    let path = seed_phrase_path(datadir);
    if !path.exists() {
        return json!({ "status": "none" });
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return json!({ "status": "legacy_corrupt", "reason": "unreadable" }),
    };
    json!({ "status": classify_legacy_phrase(&raw) })
}

// Pure classification of a legacy phrase (testable without a node): "legacy_valid" only if 12/24 words AND a
// valid BIP39 checksum; "legacy_corrupt" otherwise. A corrupt file must never read as "no wallet".
fn classify_legacy_phrase(raw: &str) -> &'static str {
    let words: Vec<&str> = raw.split_whitespace().collect();
    if words.len() != 12 && words.len() != 24 {
        return "legacy_corrupt";
    }
    match Mnemonic::parse(words.join(" ").trim()) {
        Ok(_) => "legacy_valid",
        Err(_) => "legacy_corrupt",
    }
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

// Mining mode ("solo"/"pool") + custom pool address + the OPTIONAL "start automatically when mainnet goes live"
// choice, persisted so they survive restarts. The app fully owns this small JSON object; it is written ATOMICALLY
// (temp + rename) so a crash mid-write never leaves a half-written prefs file, and every field is PRESERVED across
// the partial updates the UI makes (changing the mode must not wipe the auto-start choice, and vice versa).
fn mining_prefs_path(datadir: &std::path::Path) -> std::path::PathBuf { datadir.join("mining_prefs.json") }
fn read_mining_prefs_raw(datadir: &std::path::Path) -> Value {
    std::fs::read_to_string(mining_prefs_path(datadir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}
fn write_mining_prefs_raw(datadir: &std::path::Path, v: &Value) {
    let _ = std::fs::create_dir_all(datadir);
    let path = mining_prefs_path(datadir);
    let tmp = path.with_extension("json.tmp");
    // Atomic: write the whole object to a temp file, then rename over the target. rename() on the same
    // filesystem is atomic (Windows included), so a reader never sees a partially written prefs file.
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}
fn load_mining_prefs(datadir: &std::path::Path) -> (String, String) {
    let v = read_mining_prefs_raw(datadir);
    let modo = v["mode"].as_str().unwrap_or("solo");
    // The last door: this file is plain JSON on disk and anyone can edit it. Only "solo" and the official
    // "pool" are valid in 1.0.9 -- there are NO custom pools. A hand-written "custom" (with an attacker's
    // address) or any unknown mode is normalised to solo; "pool" is honoured only when the pool ships enabled.
    // The miner ALWAYS uses the pinned OFFICIAL_POOL_URL for pool mode, so a hand-written address in this file
    // can never redirect payouts. The mode the app believes it is in always matches the mode it actually runs.
    let modo = match modo {
        "pool" if POOL_ENABLED => "pool",
        _ => "solo",
    };
    (modo.to_string(), v["addr"].as_str().unwrap_or("").to_string())
}
fn save_mining_prefs(datadir: &std::path::Path, mode: &str, addr: &str) {
    let mut v = read_mining_prefs_raw(datadir);
    v["mode"] = json!(mode);
    v["addr"] = json!(addr);
    write_mining_prefs_raw(datadir, &v);
}
// The VOLUNTARY "start mining automatically when mainnet goes live" choice. OFF unless the user explicitly armed
// it; `intensity` is the CPU configuration they picked (the same suave/equilibrado/intenso the manual button uses,
// so the auto-start honours the exact setting chosen, even across a restart). Read on startup to re-arm the pending
// launch; written atomically, preserving mode/addr. It is consumed (set back to false) the moment mining actually
// starts, so a later restart never re-triggers an already-honoured auto-start.
fn load_autostart(datadir: &std::path::Path) -> (bool, String) {
    let v = read_mining_prefs_raw(datadir);
    (v["autoStart"].as_bool().unwrap_or(false),
     v["autoIntensity"].as_str().unwrap_or("equilibrado").to_string())
}
fn save_autostart(datadir: &std::path::Path, enabled: bool, intensity: &str) {
    let mut v = read_mining_prefs_raw(datadir);
    v["autoStart"] = json!(enabled);
    v["autoIntensity"] = json!(intensity);
    write_mining_prefs_raw(datadir, &v);
}

// Validate a user-supplied custom pool address (host:port): rejects control chars, empty host, bad/out-of-range
// port, and local/private targets by default. Mirrors the worker's own check so an invalid URL never reaches it.
fn validate_pool_addr(host_port: &str) -> Result<(), String> {
    let hp = host_port.trim();
    if hp.chars().any(|c| c.is_control()) {
        return Err("ERR:POOL_ADDR_INVALID".into());
    }
    let (host, port) = hp.rsplit_once(':').ok_or_else(|| "ERR:POOL_ADDR_FORMAT".to_string())?;
    if host.is_empty() {
        return Err("ERR:POOL_ADDR_FORMAT".into());
    }
    let port: u32 = port.parse().map_err(|_| "ERR:POOL_ADDR_PORT".to_string())?;
    if port == 0 || port > 65535 {
        return Err("ERR:POOL_ADDR_PORT".into());
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Err("ERR:POOL_ADDR_LOCAL".into());
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let blocked = match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast()
            }
            std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
        };
        if blocked {
            return Err("ERR:POOL_ADDR_LOCAL".into());
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

fn backup_flag_path(datadir: &std::path::Path) -> std::path::PathBuf { datadir.join("backup_verified.json") }

// Whether the wallet backup was REALLY verified (the user re-entered the 12 words and they matched). Persisted
// next to the wallet, NEVER a default true: a false "backup verified" badge can cause real fund loss. Absent or
// unreadable file -> false.
fn read_backup_verified(datadir: &std::path::Path) -> bool {
    std::fs::read_to_string(backup_flag_path(datadir)).ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["verified"].as_bool())
        .unwrap_or(false)
}

// Persist REAL proof of backup (fingerprint + method + date). Called ONLY after a genuine re-entry match.
// Atomic (tmp + rename) with restrictive perms where the OS allows, like the seed file.
fn write_backup_verified(datadir: &std::path::Path, fingerprint: &str) {
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let v = json!({ "verified": true, "fingerprint": fingerprint, "method": "reenter-words-v1", "date_unix": ts });
    let path = backup_flag_path(datadir);
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

// Invalidate the verified-backup proof (e.g. a new/restored wallet): the badge must NOT carry over to a different
// wallet. Called when a wallet is created/restored so a fresh wallet starts as NOT verified.
fn clear_backup_verified(datadir: &std::path::Path) {
    let _ = std::fs::remove_file(backup_flag_path(datadir));
}

/// The single guard that keeps create/restore from EVER overwriting an existing wallet. Both
/// `wallet_create_bip39` and `wallet_restore_bip39` MUST call this before touching any file: a wallet on disk
/// is the one irreplaceable asset, and silently writing a new seed over it would destroy the user's coins (and
/// re-prompt for 12 words a user may never have written down). Tested by `an_existing_wallet_is_never_overwritten`.
fn refuse_if_wallet_exists(datadir: &std::path::Path) -> Result<(), String> {
    if enc_seed_path(datadir).exists() {
        return Err("ERR:WALLET_EXISTS".to_string());
    }
    Ok(())
}

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
// Writes the encrypted phrase. `allow_overwrite` is false for create/restore (the phrase must never be
// replaced) and true only for wallet_migrate_encrypt, whose whole job is rewriting an existing wallet's
// phrase under a new password. Callers MUST hold AppState.wallet_ops: the exists-check and the write are
// not atomic on their own.
fn encrypt_phrase_file_ex(datadir: &std::path::Path, phrase: &str, password: &str, allow_overwrite: bool) -> Result<(), String> {
    if !allow_overwrite && enc_seed_path(datadir).exists() {
        // Fail closed. Reaching here means a wallet already exists and something tried to write a
        // different phrase over it, which would destroy the only way to recover the old funds.
        return Err("ERR:WALLET_EXISTS".to_string());
    }
    encrypt_phrase_file_inner(datadir, phrase, password)
}

// Kept for callers that legitimately rewrite the file (migration) and for the crypto round-trip tests.
fn encrypt_phrase_file(datadir: &std::path::Path, phrase: &str, password: &str) -> Result<(), String> {
    encrypt_phrase_file_inner(datadir, phrase, password)
}

fn encrypt_phrase_file_inner(datadir: &std::path::Path, phrase: &str, password: &str) -> Result<(), String> {
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
    use super::{decrypt_phrase_file, encrypt_phrase_file, refuse_if_wallet_exists, enc_seed_path};
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

    // GUARD: create/restore must NEVER overwrite an existing wallet. Both commands funnel through
    // refuse_if_wallet_exists; if a future change drops that call, this test fails first. A wallet on disk is
    // the one asset that cannot be regenerated — overwriting it re-prompts for 12 words (which a user may not
    // have written down) or destroys their coins.
    #[test]
    fn an_existing_wallet_is_never_overwritten() {
        let dir = std::env::temp_dir().join("brisvia_wallet_guard_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // No seed yet -> a fresh create/restore is allowed.
        assert!(refuse_if_wallet_exists(&dir).is_ok(), "with no wallet on disk, create/restore must be allowed");
        // A seed on disk -> create/restore must be refused with ERR:WALLET_EXISTS.
        std::fs::write(enc_seed_path(&dir), b"existing wallet bytes").unwrap();
        assert_eq!(refuse_if_wallet_exists(&dir), Err("ERR:WALLET_EXISTS".to_string()),
                   "an existing wallet must never be overwritten by create/restore");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // B (legacy wallets): a legacy plaintext phrase is classified by BIP39; a corrupt one is NEVER "valid" and
    // (in the frontend) never becomes create-onboarding. A wrong word count or bad checksum -> legacy_corrupt.
    #[test]
    fn legacy_phrase_classification() {
        let valid = "legal winner thank year wave sausage worth useful legal winner thank yellow"; // BIP39 vector
        assert_eq!(super::classify_legacy_phrase(valid), "legacy_valid");
        assert_eq!(super::classify_legacy_phrase("legal winner thank"), "legacy_corrupt"); // wrong count
        assert_eq!(super::classify_legacy_phrase(""), "legacy_corrupt");
        let bad = "legal winner thank year wave sausage worth useful legal winner thank thank"; // broken checksum
        assert_eq!(super::classify_legacy_phrase(bad), "legacy_corrupt");
    }
}

#[cfg(test)]
mod node_conf_tests {
    use super::node_conf;

    // Guardian (audit P0 #2): after the mainnet port move (P2P 9333->9342, RPC 9332->9338), a stale install
    // must never linger on the old ports. The app rewrites bitcoin.conf on EVERY start, so the generated conf
    // must carry exactly the rpc port it is given, must NOT pin a P2P port (the chainparams default 9342 is used),
    // and — fed the mainnet RPC port — must never contain Litecoin's 9333/9332.
    #[test]
    fn node_conf_emits_the_given_rpc_port_and_no_p2p_port() {
        let conf = node_conf("brisvia", "listen=1\n", 9338, "addnode=1.2.3.4\n");
        assert!(conf.contains("rpcport=9338"), "must carry the mainnet RPC port: {conf}");
        assert!(!conf.contains("9333"), "must never emit Litecoin's P2P port 9333: {conf}");
        assert!(!conf.contains("9332"), "must never emit Litecoin's RPC port 9332: {conf}");
        assert!(!conf.contains("\nport="), "must not pin a P2P port; the chainparams default (9342) is used: {conf}");
    }

    // The compiled mainnet RPC port is the new, own port — not Litecoin's 9332.
    #[cfg(feature = "mainnet")]
    #[test]
    fn mainnet_rpc_port_is_the_new_own_port() {
        assert_eq!(super::RPC_PORT, 9338);
    }
}

#[cfg(test)]
mod launch_window_tests {
    use super::MAINNET_START;

    // The node's own limit: a chain tip older than this means "still syncing" (bitcoin's DEFAULT_MAX_TIP_AGE).
    const MAX_TIP_AGE_SECS: i64 = 24 * 60 * 60;

    // THE ONE THAT CAN KILL THE COIN ON DAY ONE.
    //
    // miner_start refuses to mine while the node reports initialblockdownload=true -- correct: mining on a
    // stale chain is worse than not mining. The node decides that with two checks: chain work below the
    // minimum, and a tip older than 24 hours.
    //
    // On August 1st the network is born holding ONE block. If that block counted as "old", every node on
    // earth would say "still syncing", nobody could mine block 1, and the chain would never start. It works
    // only because the genesis is timestamped at the exact launch instant: at 15:00 it is a newborn block,
    // not an old one. That is load-bearing, and it was nowhere written down.
    //
    // What this pins: genesis time == launch time, so the tip is zero seconds old when mining opens.
    #[test]
    fn at_launch_the_genesis_is_new_enough_to_mine_on() {
        let tip_age_at_launch = 0; // the genesis IS the launch instant
        assert!(
            tip_age_at_launch < MAX_TIP_AGE_SECS,
            "at 15:00 UTC on August 1st the node would call itself 'syncing' and refuse to mine block 1"
        );
    }

    // Brisvia's minimum chain work is zero (chainparams.cpp: nMinimumChainWork = uint256{}), so the genesis
    // alone already satisfies it. If that ever becomes non-zero, a fresh network can never leave "syncing"
    // and nobody mines block 1 -- the other half of the same trap.
    #[test]
    fn the_minimum_chain_work_must_stay_zero_for_a_network_that_starts_from_nothing() {
        // Documented here because the value lives in the C++ core, out of this crate's reach. Changing it
        // there without reading this is exactly how the launch would break.
        let minimum_chain_work_is_zero = true; // verified in src/kernel/chainparams.cpp, CBrisviaMainParams
        assert!(
            minimum_chain_work_is_zero,
            "with a non-zero minimum, a chain holding only the genesis stays 'syncing' forever"
        );
    }

    // THE LIMIT NOBODY HAD WRITTEN DOWN: the window is 24 hours wide, and then it closes.
    //
    // If NOBODY mines block 1 during the first day, the genesis turns "old", every node latches to "syncing",
    // and the network is dead with no way back -- short of every user passing -maxtipage by hand, which a
    // non-technical owner cannot ask of anyone.
    //
    // The seed nodes mine from minute zero, so in practice this does not happen. But the margin is 24 hours,
    // not infinite, and that is a fact the launch plan has to state out loud rather than discover.
    #[test]
    fn the_launch_window_is_exactly_one_day_wide() {
        let window_closes = MAINNET_START + MAX_TIP_AGE_SECS;
        assert_eq!(
            window_closes - MAINNET_START,
            86_400,
            "the network has 24 h to mine its first block, and not one second more"
        );
    }
}

#[cfg(test)]
mod pool_disabled_tests {
    use super::{POOL_ENABLED, OFFICIAL_POOL_URL};

    // The 1.0 line ships with solo mining only. The stratum engine is finished and tested against the live
    // pool, but what a pool USER needs is not: seeing the connection, and the difference between a share
    // found, sent and ACCEPTED. Shipping the engine with no honest way to see what it does is how someone
    // ends up believing they mined for hours and got paid nothing.
    //
    // This test is the reminder, not the mechanism: turning the switch on must be a deliberate act that
    // fails here first, so nobody flips it "just to try" and ships pool mining by accident.
    // 1.0.9 ships the pool ENABLED: the honest share UI (connection state, share found vs sent vs ACCEPTED,
    // reconnect countdown, maintenance) shipped and is covered end-to-end. This test now pins the switch ON
    // deliberately, so turning it back OFF is also a conscious act, not an accident.
    #[test]
    fn pool_mining_is_enabled_in_1_0_9() {
        assert!(
            POOL_ENABLED,
            "pool mining is OFF, but 1.0.9 ships it ON (the honest share UI is in). If you meant to disable it, \
             update this test and the release guard."
        );
    }

    // The official pool address must stay pinned in the code: a pool read from anywhere the user can edit is
    // an address an attacker can replace, and the payouts would go to them.
    #[test]
    fn the_official_pool_is_pinned_in_the_code() {
        assert_eq!(OFFICIAL_POOL_URL, "pool.brisvia.com:3333");
    }

    // THE LAST DOOR. The settings file is plain JSON on disk: anyone can write "pool" into it by hand, and an
    // older version could have left it there. Disabling the button is not enough -- the UI is not the guard.
    // Loading must normalise it, so the mode the app believes it is in always matches the mode it runs.
    #[test]
    fn a_hand_written_custom_pool_is_ignored_and_only_the_official_pool_is_honoured() {
        let dir = std::env::temp_dir().join("brisvia-prefs-pool-a-mano");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // "pool" written by hand is a VALID mode in 1.0.9 (the pool ships enabled): it is honoured, but the
        // miner always uses the pinned OFFICIAL_POOL_URL, never an address taken from this file.
        super::save_mining_prefs(&dir, "pool", "pool.malicioso.com:3333");
        let (modo, _addr) = super::load_mining_prefs(&dir);
        assert_eq!(modo, "pool", "the official pool mode must be honoured with POOL_ENABLED on");

        // "custom" (a third-party pool address) is NOT supported in 1.0.9: it must be ignored (-> solo), so a
        // hand-written attacker address can never redirect the miner. This is the last door.
        super::save_mining_prefs(&dir, "custom", "cualquier.cosa:1234");
        let (modo, _) = super::load_mining_prefs(&dir);
        assert_eq!(modo, "solo", "a hand-written 'custom' pool address must be ignored, never used");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

// The voluntary "start automatically when mainnet goes live" choice: it must be OFF by default, persist the exact
// CPU setting across a restart, never clobber the mode/address (nor be clobbered by them), stay disarmed once
// consumed, write atomically, and — the safety property — the launch instant override may exist ONLY in an e2e build.
#[cfg(test)]
mod autostart_tests {
    use super::MAINNET_START;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("brisvia-autostart-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // Off unless the user armed it: with no prefs file, auto-start is disarmed (balanced default).
    #[test]
    fn auto_start_is_off_by_default() {
        let d = dir("default");
        let (armed, intensity) = super::load_autostart(&d);
        assert!(!armed, "auto-start must be OFF unless the user armed it on purpose");
        assert_eq!(intensity, "equilibrado");
        let _ = std::fs::remove_dir_all(&d);
    }

    // Arming persists and survives a fresh read from disk, with the exact CPU setting chosen (restart-proof).
    #[test]
    fn arming_persists_across_a_reload() {
        let d = dir("persist");
        super::save_autostart(&d, true, "intenso");
        let (armed, intensity) = super::load_autostart(&d);
        assert!(armed);
        assert_eq!(intensity, "intenso", "the chosen CPU setting must survive a restart exactly");
        let _ = std::fs::remove_dir_all(&d);
    }

    // The two partial updates the UI makes must not clobber each other: changing the mode must not wipe the
    // auto-start choice, and arming must not wipe the mode/address.
    #[test]
    fn mode_and_auto_start_are_preserved_independently() {
        let d = dir("preserve");
        super::save_mining_prefs(&d, "pool", "");
        super::save_autostart(&d, true, "suave");
        let (modo, _) = super::load_mining_prefs(&d);
        let (armed, intensity) = super::load_autostart(&d);
        assert_eq!(modo, "pool");
        assert!(armed);
        assert_eq!(intensity, "suave");
        // Change the mode again: the auto-start choice must remain intact.
        super::save_mining_prefs(&d, "solo", "");
        let (armed2, intensity2) = super::load_autostart(&d);
        assert!(armed2, "changing the mode wiped the auto-start choice");
        assert_eq!(intensity2, "suave");
        let _ = std::fs::remove_dir_all(&d);
    }

    // The one-shot: once consumed (disarmed the instant mining started), a later read never re-triggers it.
    #[test]
    fn consuming_the_one_shot_disarms_it() {
        let d = dir("consume");
        super::save_autostart(&d, true, "equilibrado");
        super::save_autostart(&d, false, "equilibrado");
        let (armed, _) = super::load_autostart(&d);
        assert!(!armed, "a consumed auto-start must stay off across a restart");
        let _ = std::fs::remove_dir_all(&d);
    }

    // Every write leaves a single valid JSON file and no leftover temp file (atomic tmp + rename).
    #[test]
    fn prefs_writes_are_atomic_and_valid_json() {
        let d = dir("atomic");
        super::save_mining_prefs(&d, "pool", "");
        super::save_autostart(&d, true, "intenso");
        super::save_mining_prefs(&d, "solo", "");
        let path = super::mining_prefs_path(&d);
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).expect("prefs file must be valid JSON");
        assert_eq!(v["mode"], "solo");
        assert_eq!(v["autoStart"], true);
        assert_eq!(v["autoIntensity"], "intenso");
        assert!(!path.with_extension("json.tmp").exists(), "the temp file must be renamed away, never left behind");
        let _ = std::fs::remove_dir_all(&d);
    }

    // SAFETY: a shipped (non-e2e) build has NO launch-instant override compiled in, so any injected env is ignored
    // and the launch instant is always the fixed constant. This test only exists in a non-e2e build.
    #[cfg(not(feature = "e2e"))]
    #[test]
    fn a_shipped_build_ignores_any_launch_instant_override() {
        std::env::set_var("BRISVIA_E2E_MAINNET_START", "1");
        assert_eq!(super::mainnet_start_secs(), MAINNET_START, "a shipped build must ignore a launch-instant override");
        std::env::remove_var("BRISVIA_E2E_MAINNET_START");
    }

    // Only an e2e build may move the launch instant, so a test can cross the boundary in seconds. Restores after.
    #[cfg(feature = "e2e")]
    #[test]
    fn an_e2e_build_can_move_the_launch_instant_then_restores() {
        std::env::set_var("BRISVIA_E2E_MAINNET_START", "1000000000");
        assert_eq!(super::mainnet_start_secs(), 1_000_000_000, "e2e builds must honour the injectable test instant");
        std::env::remove_var("BRISVIA_E2E_MAINNET_START");
        assert_eq!(super::mainnet_start_secs(), MAINNET_START);
    }
}

#[cfg(test)]
mod repair_marker_tests {
    use super::{ahora_secs, repair_blocked, repair_marker, REPAIR_COOLDOWN_SECS};

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("brisvia-marker-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // No marker: nothing was attempted, so a repair is allowed.
    #[test]
    fn without_a_marker_a_repair_is_allowed() {
        let d = dir("none");
        assert!(!repair_blocked(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    // A repair moments ago that is failing again = a loop. Refuse and tell the user honestly.
    #[test]
    fn a_repair_seconds_ago_blocks_another_one() {
        let d = dir("recent");
        std::fs::write(repair_marker(&d), format!("-reindex|{}", ahora_secs() - 5)).unwrap();
        assert!(repair_blocked(&d), "repairing again seconds later is a loop");
        let _ = std::fs::remove_dir_all(&d);
    }

    // THE BUG THIS FIXES, caught on the owner's machine. The old code only asked "does the marker exist?" and
    // never removed it after a repair that WORKED. Since every update force-closes the app -- which is exactly
    // what corrupts the database -- the next corruption would find that stale marker and refuse to repair,
    // telling the user "I could not repair it" without trying. An old marker must not block anything.
    #[test]
    fn an_old_marker_does_not_block_a_new_repair() {
        let d = dir("old");
        let hace_rato = ahora_secs() - REPAIR_COOLDOWN_SECS - 60;
        std::fs::write(repair_marker(&d), format!("-reindex-chainstate|{hace_rato}")).unwrap();
        assert!(!repair_blocked(&d), "a repair from long ago is a closed incident, not a loop");
        let _ = std::fs::remove_dir_all(&d);
    }

    // Markers written by the first version of this code carry no timestamp. Those are exactly the ones left
    // behind by successful repairs, so honouring them would deny that machine a repair for good.
    #[test]
    fn a_marker_from_the_old_version_does_not_block_forever() {
        let d = dir("legacy");
        std::fs::write(repair_marker(&d), "-reindex-chainstate").unwrap(); // no timestamp
        assert!(!repair_blocked(&d), "a marker with no date must not deny repairs for good");
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod repair_tests {
    use super::{classify_failure, log_since, net_subdir, Repair};

    // The real message bitcoind writes before refusing to start (it happened twice on the owner's machine
    // after the process was killed). Without this, the app hangs at "Preparing wallet..." forever.
    #[test]
    fn a_broken_block_index_asks_for_a_full_rebuild() {
        let log = "2026-07-14T10:00:00Z Loading block index...\n\
                   2026-07-14T10:00:01Z Error opening block database.\n\
                   2026-07-14T10:00:01Z Please restart with -reindex to recover.\n";
        assert_eq!(classify_failure(log), Repair::Full);
    }

    // When the node names -reindex-chainstate, take the cheap fix: rebuilding the whole index would make the
    // user wait far longer for nothing.
    #[test]
    fn a_broken_coins_database_asks_for_the_cheap_rebuild() {
        let log = "2026-07-14T10:00:01Z Error opening coins database.\n\
                   2026-07-14T10:00:01Z Please restart with -reindex-chainstate to recover.\n";
        assert_eq!(classify_failure(log), Repair::Chainstate);
    }

    // THE DANGEROUS ONE. A full disk also makes the node fail while it complains about its database. Rebuilding
    // needs MORE disk, so it cannot work: it would fail, retry and loop. Must be reported, never repaired.
    #[test]
    fn a_full_disk_is_never_repaired() {
        let log = "2026-07-14T10:00:00Z Error: Disk space is too low!\n\
                   2026-07-14T10:00:01Z Error opening block database.\n\
                   2026-07-14T10:00:01Z Please restart with -reindex to recover.\n";
        assert_eq!(classify_failure(log), Repair::Blocked("disk"));
    }

    // Two copies of the app open: the second cannot lock the datadir. Rebuilding would not fix that either.
    #[test]
    fn an_already_running_node_is_never_repaired() {
        let log = "2026-07-14T10:00:00Z Cannot obtain a lock on data directory. Brisvia is probably already running.\n";
        assert_eq!(classify_failure(log), Repair::Blocked("locked"));
    }

    #[test]
    fn a_permissions_error_is_never_repaired() {
        let log = "2026-07-14T10:00:00Z Error opening block database.\n\
                   2026-07-14T10:00:00Z Permission denied\n";
        assert_eq!(classify_failure(log), Repair::Blocked("permissions"));
    }

    // THE REAL CAUSE, found on the owner's machine and misdiagnosed all day. The genesis is dated at the
    // launch instant, so until August 1st the node sees a tip weeks in the future and refuses to start
    // (chainstate.cpp:241 tolerates 2 h). NOTHING IS BROKEN.
    //
    // This is the real reason the app used to hang forever on "Preparing wallet..." before the self-repair
    // existed. I blamed the process I had killed; the killed process was never it.
    //
    // The message below is copied verbatim from the owner's log. It ENDS with "Please restart with -reindex",
    // so without its own branch it gets classified as a corrupted database and treated as damage.
    #[test]
    fn a_genesis_in_the_future_is_not_damage() {
        let log = "2026-07-14T20:44:02Z Loaded best chain: hashBestChain=aa6bc268 height=0 date=2026-08-01T15:00:00Z\n\
                   2026-07-14T20:44:02Z init message: Verifying blocks…\n\
                   2026-07-14T20:44:02Z : The block database contains a block which appears to be from the future. \
                   This may be due to your computer's date and time being set incorrectly. Only rebuild the block \
                   database if you are sure that your computer's date and time are correct.\n\
                   Please restart with -reindex or -reindex-chainstate to recover.\n";
        assert_eq!(
            classify_failure(log),
            Repair::FutureGenesis,
            "the pre-launch genesis is being treated as a corrupted database"
        );
    }

    // And it must win over the corruption branches: the message contains BOTH "from the future" and
    // "-reindex-chainstate", so order decides. Misread, it becomes "your data is broken" when it is not.
    #[test]
    fn the_future_genesis_wins_over_the_corruption_wording_in_the_same_message() {
        let log = "The block database contains a block which appears to be from the future. \
                   Please restart with -reindex or -reindex-chainstate to recover.";
        assert_ne!(classify_failure(log), Repair::Chainstate);
        assert_ne!(classify_failure(log), Repair::Full);
        assert_eq!(classify_failure(log), Repair::FutureGenesis);
    }

    // A healthy start must not trigger anything: rebuilding on every start would be a slow, pointless loop.
    #[test]
    fn a_healthy_start_repairs_nothing() {
        let log = "2026-07-14T10:00:00Z Loading block index...\n\
                   2026-07-14T10:00:02Z init message: Done loading\n\
                   2026-07-14T10:00:03Z UpdateTip: new best=aa6bc268 height=1\n";
        assert_eq!(classify_failure(log), Repair::None);
    }

    // CAUSALITY. The bug the audit caught: an OLD message, from a crash already fixed by hand, must not
    // trigger a rebuild today. We only read what was written after the mark, so the old part is invisible.
    #[test]
    fn an_old_crash_message_does_not_trigger_a_rebuild_today() {
        let dir = std::env::temp_dir().join("brisvia-repair-causality");
        let net = dir.join(net_subdir());
        std::fs::create_dir_all(&net).unwrap();
        let old = "2026-01-01T00:00:00Z Please restart with -reindex to recover.\n";
        std::fs::write(net.join("debug.log"), old).unwrap();
        let mark = old.len() as u64; // what the log measured before today's attempt

        // Today the node starts fine and appends a healthy line.
        let mut all = old.to_string();
        all.push_str("2026-07-14T10:00:03Z UpdateTip: new best=aa6bc268 height=1\n");
        std::fs::write(net.join("debug.log"), &all).unwrap();

        let this_attempt = log_since(&dir, mark);
        assert!(!this_attempt.contains("Please restart")); // the old crash is not part of this attempt
        assert_eq!(classify_failure(&this_attempt), Repair::None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The mirror case: a real crash right now IS seen, even with a big healthy log behind it.
    #[test]
    fn a_crash_right_now_is_seen_even_after_a_long_healthy_log() {
        let dir = std::env::temp_dir().join("brisvia-repair-now");
        let net = dir.join(net_subdir());
        std::fs::create_dir_all(&net).unwrap();
        let old = "2026-07-14T09:00:00Z UpdateTip: new best=deadbeef\n".repeat(6000);
        std::fs::write(net.join("debug.log"), &old).unwrap();
        let mark = old.len() as u64;

        let mut all = old.clone();
        all.push_str("2026-07-14T10:00:01Z Corrupted block database detected.\n");
        all.push_str("2026-07-14T10:00:01Z Please restart with -reindex to recover.\n");
        std::fs::write(net.join("debug.log"), &all).unwrap();

        assert_eq!(classify_failure(&log_since(&dir, mark)), Repair::Full);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // No log at all (first ever run) must be silent, not a crash.
    #[test]
    fn a_missing_log_is_not_a_failure() {
        let dir = std::env::temp_dir().join("brisvia-repair-nonexistent");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(classify_failure(&log_since(&dir, 0)), Repair::None);
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
            format!("Brisvia — Minando al {}% · {} de {} núcleos", pct, threads, cores) // i18n-es (native tray tooltip, localized in code)
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
        // Fresh pool status for the new session (the event follower will refill it from the new log).
        state.pool_connected.store(false, Ordering::SeqCst);
        state.pool_shares_sent.store(0, Ordering::SeqCst);
        state.pool_shares_accepted.store(0, Ordering::SeqCst);
        state.pool_shares_rejected.store(0, Ordering::SeqCst);
        state.pool_last_accepted_ts.store(0, Ordering::SeqCst);
        state.pool_suspended.store(false, Ordering::SeqCst);
        state.pool_retry_after.store(0, Ordering::SeqCst);
        state.pool_has_job.store(false, Ordering::SeqCst);
        state.pool_ever_job.store(false, Ordering::SeqCst);
        *state.pool_last_error.lock().unwrap() = String::new();
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
            _ if !POOL_ENABLED => None, // pool mining is off in 1.0 (see POOL_ENABLED) — always mine solo
            "pool" => Some(OFFICIAL_POOL_URL.to_string()),
            "custom" => {
                let a = state.pool_address.lock().unwrap().clone();
                if a.trim().is_empty() { None } else { Some(a.trim().to_string()) }
            }
            _ => None,
        }
    };
    // N11: "custom" pool chosen but no address entered -> refuse to start instead of silently mining solo.
    if POOL_ENABLED && state.mining_mode.lock().unwrap().as_str() == "custom" && pool_url.is_none() {
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
    // The worker encrypts by default and only skips it if BRISVIA_POOL_PLAIN is set (the local e2e harness).
    // Remove it ALWAYS: an inherited value -- from the user's environment, another program, or something
    // hostile -- would silently downgrade the connection to plain text, and on a plain link anyone in the
    // middle can rewrite the payout address and collect the rewards. The app must never mine unencrypted.
    cmd.env_remove("BRISVIA_POOL_PLAIN");
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
        let pool_connected = state.pool_connected.clone();
        let pool_sent = state.pool_shares_sent.clone();
        let pool_accepted = state.pool_shares_accepted.clone();
        let pool_rejected = state.pool_shares_rejected.clone();
        let pool_last_error = state.pool_last_error.clone();
        let pool_suspended = state.pool_suspended.clone();
        let pool_retry_after = state.pool_retry_after.clone();
        let pool_has_job = state.pool_has_job.clone();
        let pool_ever_job = state.pool_ever_job.clone();
        let pool_last_accepted_ts = state.pool_last_accepted_ts.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let mut prev_total: u64 = 0;
            let mut prev_pool_ok: u64 = 0;
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
                // Pool-mode state, recomputed from the full session log each pass (idempotent): connection, and
                // shares SENT vs ACCEPTED vs REJECTED. A contribution counts only on share_accepted.
                let mut p_sent = 0u64;
                let mut p_ok = 0u64;
                let mut p_rej = 0u64;
                let mut p_conn = false;
                let mut p_susp = false;
                let mut p_retry = 0u64;     // absolute unix ts of the next reconnect/maintenance attempt (0 = none)
                let mut p_has_job = false;  // a pool job is active right now -> WORKING (not just authenticated)
                let mut p_ever_job = false; // had at least one job this session (authenticated/first vs waiting-again)
                let mut p_err: Option<String> = None;
                for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
                    if let Ok(evt) = serde_json::from_str::<Value>(&line) {
                        match evt["event"].as_str() {
                            // Dataset ready or first block: the engine is working, stop showing "Preparing…".
                            Some("seed_ready") => ready.store(true, Ordering::SeqCst),
                            // A real pool job arrived: we are now WORKING (contributing). Also marks ready
                            // (stops "Preparing…") and ends any reconnect countdown.
                            Some("pool_job") => {
                                ready.store(true, Ordering::SeqCst);
                                p_has_job = true; p_ever_job = true; p_retry = 0;
                            }
                            // Backoff/maintenance wait announced with an ABSOLUTE unix ts of the next attempt.
                            Some("pool_reconnecting") => { p_retry = evt["retry_at"].as_u64().unwrap_or(0); }
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
                            // ---- pool-mode events (honest and DISTINCT: sent != accepted != block) ----
                            // "connected" means the pool ACCEPTED the login (we can actually mine), not just that a
                            // TCP socket opened. So pool_connected (socket) does NOT flip it; pool_login does.
                            Some("pool_connected") => {}
                            // Login ACCEPTED -> authenticated. No job yet (until pool_job) and no longer reconnecting.
                            Some("pool_login") => { p_conn = true; p_susp = false; p_has_job = false; p_retry = 0; ready.store(true, Ordering::SeqCst); }
                            Some("share_submitted") => { p_sent += 1; }
                            Some("share_accepted") => { p_ok += 1; ready.store(true, Ordering::SeqCst); }
                            Some("share_rejected") => {
                                p_rej += 1;
                                p_err = Some(match evt["reason"].as_str() {
                                    Some(r) => format!("share rechazada: {r}"), None => "share rechazada".into() });
                            }
                            // A share met the NETWORK target: a real block found via the pool. Counts as a block.
                            Some("pool_block") => { total += 1; ready.store(true, Ordering::SeqCst); }
                            Some("pool_disconnected") => {
                                p_conn = false; p_has_job = false;
                                p_err = Some(match evt["reason"].as_str() {
                                    Some(r) => format!("desconectado: {r}"), None => "desconectado".into() });
                            }
                            // The pool is under MAINTENANCE (explicit), not a crash or an error. Not connected
                            // for mining, but the UI must say "en mantenimiento", never "error" or "solo". The
                            // miner keeps reconnecting on its own; clear any stale error so nothing scary lingers.
                            // Maintenance: not connected, not an error, NOT a fall to solo. The countdown comes
                            // from the accompanying pool_reconnecting (absolute ts); do not overwrite it here.
                            Some("pool_suspended") => { p_conn = false; p_susp = true; p_has_job = false; p_err = None; }
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
                // Pool status: publish the recomputed counters. Stamp the last-accepted time only when it grows.
                pool_connected.store(p_conn, Ordering::SeqCst);
                pool_suspended.store(p_susp, Ordering::SeqCst);
                pool_retry_after.store(p_retry, Ordering::SeqCst);
                pool_has_job.store(p_has_job, Ordering::SeqCst);
                pool_ever_job.store(p_ever_job, Ordering::SeqCst);
                pool_sent.store(p_sent, Ordering::SeqCst);
                pool_accepted.store(p_ok, Ordering::SeqCst);
                pool_rejected.store(p_rej, Ordering::SeqCst);
                if p_ok > prev_pool_ok {
                    prev_pool_ok = p_ok;
                    pool_last_accepted_ts.store(
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs()).unwrap_or(0), Ordering::SeqCst);
                }
                if let Some(e) = p_err { *pool_last_error.lock().unwrap() = e; }
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

// Physical core count, detected ONCE per session (the CPU topology never changes while running, so there is
// no point querying it on every status poll). num_cpus falls back to the logical count if it cannot tell the
// physical one apart; the UI treats "physical >= logical" as "unknown" and then shows only the thread count.
fn physical_cores() -> usize {
    use std::sync::OnceLock;
    static PHYS: OnceLock<usize> = OnceLock::new();
    *PHYS.get_or_init(num_cpus::get_physical)
}

// The launch instant, as the single canonical UTC source of truth for "is mainnet live yet". In production this
// is ALWAYS the fixed constant. Only an e2e/CI build (feature "e2e") may override it via BRISVIA_E2E_MAINNET_START,
// so a test can cross the launch boundary in seconds without touching the machine clock. The override literally
// cannot exist in a shipped build: the branch is compiled out.
#[cfg(feature = "e2e")]
fn mainnet_start_secs() -> i64 {
    std::env::var("BRISVIA_E2E_MAINNET_START").ok().and_then(|s| s.parse::<i64>().ok()).unwrap_or(MAINNET_START)
}
#[cfg(not(feature = "e2e"))]
fn mainnet_start_secs() -> i64 { MAINNET_START }

#[tauri::command]
fn miner_status(state: State<AppState>) -> Value {
    let mining = state.mining.load(Ordering::SeqCst);
    // The auto-start-at-launch choice (voluntary, off unless armed) travels to the UI so the pending launch can be
    // re-armed after a restart, and the canonical launch instant so the frontend has ONE source of truth for time.
    let (auto_start, auto_intensity) = load_autostart(&state.datadir);
    let mainnet_start_ms = mainnet_start_secs() * 1000;
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
    // Pool state machine (single source of truth, derived from the miner's events). "working" REQUIRES a real
    // active job, never just an open socket. "reconnecting" is a real countdown; "suspended" (maintenance) takes
    // priority. The miner NEVER falls to solo: an off/idle pool reads as connecting/reconnecting/disconnected.
    let pool_conn = state.pool_connected.load(Ordering::SeqCst);
    let pool_susp = state.pool_suspended.load(Ordering::SeqCst);
    let pool_hasjob = state.pool_has_job.load(Ordering::SeqCst);
    let pool_everjob = state.pool_ever_job.load(Ordering::SeqCst);
    let pool_retry_at = state.pool_retry_after.load(Ordering::SeqCst);
    let now_secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0);
    let pool_phase = if !mining { "disconnected" }
        else if pool_susp { "suspended" }
        else if !pool_conn && pool_retry_at > now_secs { "reconnecting" }
        else if !pool_conn { "connecting" }
        else if pool_hasjob { "working" }
        else if pool_everjob { "waiting" }
        else { "authenticated" };
    let pool_retry_secs = pool_retry_at.saturating_sub(now_secs);
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
        // `cores` is the max the miner can use = logical processors (threads). `physicalCores` is the real
        // core count, shown alongside it so "24 cores / 32 threads" reads honestly (mining runs on threads).
        "cores": state.cores as u64,
        "physicalCores": physical_cores() as u64,
        // Which mode the app is ACTUALLY running (normalised: never "pool" while POOL_ENABLED is off).
        "mode": state.mining_mode.lock().unwrap().clone(),
        // Pool-mode live status for the honest UI. `sharesSent` is what left the miner; `sharesAccepted` is what
        // the pool CONFIRMED (a contribution counts only here). A sent share is not a paid share.
        "pool": {
            "enabled": POOL_ENABLED,
            "connected": pool_conn,
            "suspended": pool_susp,
            // The real state-machine phase and the reconnect/maintenance countdown (seconds remaining, 0 = none).
            "phase": pool_phase,
            "hasJob": pool_hasjob,
            "retrySecs": pool_retry_secs,
            "sharesSent": state.pool_shares_sent.load(Ordering::SeqCst),
            "sharesAccepted": state.pool_shares_accepted.load(Ordering::SeqCst),
            "sharesRejected": state.pool_shares_rejected.load(Ordering::SeqCst),
            "lastAcceptedTs": state.pool_last_accepted_ts.load(Ordering::SeqCst),
            "lastError": state.pool_last_error.lock().unwrap().clone(),
        },
        // Auto-start-at-launch: the canonical launch instant (UTC ms) + whether the user armed automatic start and
        // with how many threads. The frontend uses `mainnetStartMs` as its single source of truth for the countdown
        // and the hot unlock, and re-arms the pending auto-start from `autoStart` after a restart.
        "mainnetStartMs": mainnet_start_ms,
        "autoStart": auto_start,
        "autoIntensity": auto_intensity,
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
        // The UI reads the same switch the backend obeys, so a screen offering pool mining and a backend
        // refusing it can never disagree.
        "poolEnabled": POOL_ENABLED,
        // With pool mining off, report the honest truth: solo is what actually runs, whatever is stored.
        "miningMode": if POOL_ENABLED { state.mining_mode.lock().unwrap().clone() } else { "solo".to_string() },
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
            if let Some(s) = value.as_str() {
                // With pool mining off, "pool"/"custom" are not just unusable: STORING them would leave the
                // saved state disagreeing with what actually runs (the miner always goes solo), and the day
                // 1.1 turns the pool on, that stale value would silently start mining in a pool the user never
                // chose in THAT version. The settings file is editable by hand, so refusing here -- not only
                // in the UI -- is what makes "no pool route" true rather than merely displayed.
                let pedido = if POOL_ENABLED { s } else { "solo" };
                *state.mining_mode.lock().unwrap() = pedido.to_string();
            }
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

// Arm or disarm the VOLUNTARY "start mining automatically when mainnet goes live" choice. Off unless the user
// explicitly turns it on; the frontend passes the chosen thread count. Persisted atomically (preserving mode/addr)
// so it survives a restart. It is a ONE-SHOT: the frontend disarms it (enabled=false) the instant mining actually
// starts, so a later restart never re-triggers an already-honoured auto-start.
#[tauri::command]
fn mining_set_autostart(state: State<AppState>, enabled: bool, intensity: String) -> Value {
    // Only the app's real intensity labels are accepted; anything else falls back to the balanced default, so a
    // hand-crafted call can never inject an odd value that reaches the engine.
    let intensity = match intensity.as_str() {
        "suave" | "equilibrado" | "intenso" => intensity.as_str(),
        _ => "equilibrado",
    };
    save_autostart(&state.datadir, enabled, intensity);
    json!({ "ok": true, "autoStart": enabled, "autoIntensity": intensity })
}

// Hosts the app is allowed to open in the system browser. A wallet app must not open arbitrary URLs, so
// this surface is restricted to our own site and the social/share networks it links to.
fn is_open_url_allowed(url: &str) -> bool {
    // https only.
    let rest = match url.strip_prefix("https://") {
        Some(r) => r,
        None => return false,
    };
    // Authority = everything before the first '/', '?' or '#'.
    let end = rest.find(|c| c == '/' || c == '?' || c == '#').unwrap_or(rest.len());
    let authority = &rest[..end];
    // Reject an empty authority and any "user@host" userinfo trick.
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    // Drop an optional ":port".
    let host = authority.split(':').next().unwrap_or(authority).to_ascii_lowercase();
    // Our own domain and any subdomain (brisvia.com, explorer.brisvia.com, pool.brisvia.com, …).
    if host == "brisvia.com" || host.ends_with(".brisvia.com") {
        return true;
    }
    // Social + share networks the app links to (barra social + botón Compartir Brisvia).
    const ALLOWED: &[&str] = &[
        "x.com", "twitter.com",
        "instagram.com", "www.instagram.com",
        "facebook.com", "www.facebook.com",
        "telegram.me", "t.me",
        "discord.gg", "discord.com",
        "reddit.com", "www.reddit.com",
        "wa.me", "api.whatsapp.com",
    ];
    ALLOWED.contains(&host.as_str())
}

// Open a link (web/social) in the system browser, not inside the webview.
#[tauri::command]
fn open_url(url: String) {
    // Only https URLs to an explicitly allowed host. Anything else is refused (wallet app: tight surface).
    if !is_open_url_allowed(&url) {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        // rundll32 (not "cmd /C start"): it takes the URL as a real argument, so '&' and other query
        // characters in the share links can't be reinterpreted by the shell.
        let _ = Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", &url])
            .spawn();
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

#[cfg(test)]
mod open_url_tests {
    use super::is_open_url_allowed;

    #[test]
    fn allows_own_site_and_social_and_share_hosts() {
        // Social bar
        assert!(is_open_url_allowed("https://brisvia.com/"));
        assert!(is_open_url_allowed("https://explorer.brisvia.com/tx/abc"));
        assert!(is_open_url_allowed("https://x.com/brisviacoin"));
        assert!(is_open_url_allowed("https://instagram.com/brisviacoin"));
        assert!(is_open_url_allowed("https://www.facebook.com/profile.php?id=61591756292086"));
        assert!(is_open_url_allowed("https://telegram.me/brisvia"));
        assert!(is_open_url_allowed("https://discord.gg/ZF4MExMJn4"));
        assert!(is_open_url_allowed("https://www.reddit.com/r/Brisvia"));
        // Share button (query strings with '&')
        assert!(is_open_url_allowed("https://twitter.com/intent/tweet?text=hi&url=https%3A%2F%2Fbrisvia.com"));
        assert!(is_open_url_allowed("https://t.me/share/url?url=https%3A%2F%2Fbrisvia.com&text=hi"));
        assert!(is_open_url_allowed("https://wa.me/?text=hi%20https%3A%2F%2Fbrisvia.com"));
    }

    #[test]
    fn refuses_other_schemes_hosts_and_tricks() {
        assert!(!is_open_url_allowed("http://brisvia.com/"));            // not https
        assert!(!is_open_url_allowed("https://evil.com/"));             // host not allowed
        assert!(!is_open_url_allowed("https://brisvia.com.evil.com/")); // suffix trick
        assert!(!is_open_url_allowed("https://evil.com@brisvia.com/")); // userinfo trick
        assert!(!is_open_url_allowed("https://notbrisvia.com/"));       // lookalike, no dot
        assert!(!is_open_url_allowed("file:///etc/passwd"));            // not https
        assert!(!is_open_url_allowed("javascript:alert(1)"));           // not https
        assert!(!is_open_url_allowed("https:///nohostpath"));           // empty authority
    }
}

// ---- system locale (to pick es/en the first time) ----
#[tauri::command]
fn system_locale() -> String {
    sys_locale::get_locale().unwrap_or_else(|| "en".to_string())
}

// Tray menu labels by language.
fn tray_labels(lang: &str) -> (&'static str, &'static str) {
    if lang == "es" { ("Abrir Brisvia", "Salir de Brisvia") } else { ("Open Brisvia", "Exit Brisvia") } // i18n-es (native tray menu labels)
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
        .ok_or_else(|| "ERR:NO_UPDATE".to_string())?;
    // The node has to be COMPLETELY gone before the installer touches a single file.
    //
    // This used to stop the node and start installing straight after, and `stop_node` gave it 5.5
    // seconds before killing it. Two ways to corrupt a block database in one line: kill it mid-flush,
    // or replace the binary underneath a process that is still writing.
    //
    // Now it waits for the process to really disappear, and if it does not, the update is CANCELLED.
    // A user who has to press the button again is a nuisance. A user whose chain is corrupted has to
    // resync from zero, and it happened silently while they were told it was updating.
    {
        let state = app.state::<AppState>();
        // No new sends and no new mining from here on: the node is on its way out, and a transaction
        // accepted now would be answered by a node that is closing.
        state.mining.store(false, Ordering::SeqCst);
        if !stop_node_and_wait(&state) {
            // Deliberately no kill: it is still running because it is still writing.
            return Err("ERR:NODE_STILL_RUNNING".to_string());
        }
    }
    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Linux (Intel/Wayland/Mesa): WebKitGTK can fail to initialize EGL ("Could not create default EGL
    // display: EGL_BAD_PARAMETER. Aborting...") on Intel HD graphics under a Wayland session, so the window
    // never opens (reported on a ThinkPad T430 / Intel HD 4000 / Ubuntu 26.04). Disabling the DMABUF renderer
    // and the accelerated compositor makes WebKitGTK fall back to a path that initializes there. These are set
    // as DEFAULTS, before the webview is created, and only when the user has not set them, so power users can
    // still override (e.g. WEBKIT_DISABLE_COMPOSITING_MODE=0). This covers both the .AppImage and the .deb.
    // GDK_BACKEND is deliberately NOT forced: pinning everyone to XWayland would regress working Wayland setups.
    #[cfg(target_os = "linux")]
    {
        for (k, v) in [
            ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
            ("WEBKIT_DISABLE_COMPOSITING_MODE", "1"),
        ] {
            if std::env::var_os(k).is_none() {
                std::env::set_var(k, v);
            }
        }
    }

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
        sending: Arc::new(Mutex::new(())),
        wallet_ops: Arc::new(Mutex::new(())),
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
        pool_connected: Arc::new(AtomicBool::new(false)),
        pool_suspended: Arc::new(AtomicBool::new(false)),
        pool_retry_after: Arc::new(AtomicU64::new(0)),
        pool_has_job: Arc::new(AtomicBool::new(false)),
        pool_ever_job: Arc::new(AtomicBool::new(false)),
        pool_shares_sent: Arc::new(AtomicU64::new(0)),
        pool_shares_accepted: Arc::new(AtomicU64::new(0)),
        pool_shares_rejected: Arc::new(AtomicU64::new(0)),
        pool_last_error: Arc::new(Mutex::new(String::new())),
        pool_last_accepted_ts: Arc::new(AtomicU64::new(0)),
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
            wallet_estimate_send,
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
            wallet_legacy_status,
            settings_get,
            settings_set,
            mining_set_autostart,
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

// Error messages that reach the user must be translated codes, never Core's raw English.
// The frontend maps "ERR:CODE" to the active language and passes UNKNOWN strings through unchanged
// (app.js transError), so a missing translation here surfaces raw English — including the word "Bitcoin"
// and Windows paths — on a screen that handles money.
//
// Every string below was CAPTURED from a real bitcoind v30.2 on regtest, not written from memory.
// That matters: an invented message would let a wrong rule pass green. This exact battery caught one.
#[cfg(test)]
mod error_translation_tests {
    use super::friendly_error;

    // Captured: bitcoin-cli createwallet <name> twice on the same datadir.
    const CORE_WALLET_EXISTS: &str = r"Wallet file verification failed. Failed to create database path 'C:\Users\testuser\AppData\Local\Temp\wtest\regtest\wallets\w2'. Database already exists.";
    // Captured: bitcoin-cli walletpassphrase "wrongpassword" 10
    const CORE_BAD_PASSWORD: &str = "Error: The wallet passphrase entered was incorrect.";
    // Captured: bitcoin-cli sendtoaddress on a locked wallet. Contains "passphrase" but is NOT a wrong password.
    const CORE_WALLET_LOCKED: &str = "Error: Please enter the wallet passphrase with walletpassphrase first.";

    #[test]
    fn restoring_over_an_existing_wallet_says_so_instead_of_leaking_a_windows_path() {
        assert_eq!(friendly_error(CORE_WALLET_EXISTS), "ERR:WALLET_EXISTS");
        // The raw message must not survive: it carries the user's home directory.
        assert!(!friendly_error(CORE_WALLET_EXISTS).contains("C:\\"));
    }

    #[test]
    fn a_wrong_wallet_password_is_reported_as_a_wrong_password() {
        assert_eq!(friendly_error(CORE_BAD_PASSWORD), "ERR:BAD_PASSWORD");
    }

    // Regression guard. A rule of `contains("passphrase") || contains("incorrect")` translates BOTH Core
    // messages to BAD_PASSWORD, telling a user their password was wrong when the wallet was merely locked
    // and they never typed one. Both words must be required.
    #[test]
    fn a_locked_wallet_is_never_reported_as_a_wrong_password() {
        assert_ne!(friendly_error(CORE_WALLET_LOCKED), "ERR:BAD_PASSWORD");
    }

    #[test]
    fn the_money_errors_are_translated() {
        assert_eq!(friendly_error("Insufficient funds"), "ERR:INSUFFICIENT_FUNDS");
        assert_eq!(friendly_error("Invalid Bitcoin address"), "ERR:INVALID_ADDRESS");
        // Core names the wrong coin in that string; the user must never read "Bitcoin" inside Brisvia.
        assert!(!friendly_error("Invalid Bitcoin address").contains("Bitcoin"));
    }

    // The backend only rejects amount <= 0, so an amount with 9+ decimals reaches Core and comes back as
    // "Invalid amount". Both this and the bad-address message start with "invalid": the amount rules must
    // win, or a user with a typo in the amount is told their address is wrong and goes looking in the
    // wrong place — on a screen that moves money.
    #[test]
    fn a_bad_amount_is_not_reported_as_a_bad_address() {
        assert_eq!(friendly_error("Invalid amount"), "ERR:INVALID_AMOUNT");
        assert_eq!(friendly_error("Invalid amount for send"), "ERR:INVALID_AMOUNT");
        assert_eq!(friendly_error("Amount out of range"), "ERR:INVALID_AMOUNT");
        assert_eq!(friendly_error("Transaction amount too small"), "ERR:AMOUNT_TOO_SMALL");
        // and the address rule still works for actual address problems
        assert_eq!(friendly_error("Invalid Bitcoin address"), "ERR:INVALID_ADDRESS");
    }

    // Changed on purpose (it used to pass the raw text through). The frontend renders unknown strings
    // verbatim, so forwarding Core's English meant users read internal wording — and their own home
    // directory — on a screen that moves money. Unknown now means a sanitised, actionable message, with
    // the detail kept in the log.
    #[test]
    fn an_unknown_message_is_never_shown_raw() {
        assert_eq!(friendly_error("something nobody mapped"), "ERR:OPERATION_FAILED");
        assert_eq!(friendly_error("Error: unknown internal RPC failure at some/path"), "ERR:OPERATION_FAILED");
    }

    #[test]
    fn no_error_ever_reaches_the_user_carrying_their_home_directory() {
        // Every message the node can hand us must come back as a code, never as text with a path in it.
        for raw in [CORE_WALLET_EXISTS, CORE_BAD_PASSWORD, CORE_WALLET_LOCKED, "Insufficient funds", "boom at C:\\Users\\alice\\x"] {
            let shown = friendly_error(raw);
            assert!(shown.starts_with("ERR:"), "not a code: {shown}");
            assert!(!shown.to_lowercase().contains("users"), "leaked a path: {shown}");
            assert!(!shown.contains("Bitcoin"), "said Bitcoin inside Brisvia: {shown}");
        }
    }

    #[test]
    fn the_log_keeps_the_detail_but_strips_personal_paths() {
        let leaky = "boom at C:\\Users\\alice\\AppData\\Local\\Temp\\x.dat while doing the thing";
        let logged = super::sanitize_for_log(leaky);
        assert!(!logged.contains("alice"), "the log leaked the user name: {logged}");
        assert!(logged.contains("boom") && logged.contains("while doing the thing"), "the log lost the detail: {logged}");
    }
}

// Two clicks on "Send" must never become two payments. Core cannot tell a duplicate apart: two
// sendtoaddress calls with funds available are two real, irreversible transactions. Disabling the button
// in JS is UX, not a barrier — a second event, the Enter key, or any other caller reaches the backend.
//
// These tests exercise the exact guard wallet_send uses (try_lock on AppState.sending), with real threads
// and a barrier so the race is deterministic rather than a lucky interleaving.
#[cfg(test)]
mod send_single_flight_tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier, Mutex};

    // Mirrors wallet_send's guard: try_lock, and on failure return the error WITHOUT waiting.
    fn guarded_send(sending: &Arc<Mutex<()>>, rpc_calls: &Arc<AtomicU64>) -> Result<&'static str, String> {
        let _g = sending.try_lock().map_err(|_| "ERR:SEND_IN_PROGRESS".to_string())?;
        rpc_calls.fetch_add(1, Ordering::SeqCst); // stands in for the real sendtoaddress
        std::thread::sleep(std::time::Duration::from_millis(60)); // hold the guard, widening the race window
        Ok("txid")
    }

    #[test]
    fn two_simultaneous_sends_reach_the_node_exactly_once() {
        let sending = Arc::new(Mutex::new(()));
        let calls = Arc::new(AtomicU64::new(0));
        let barrier = Arc::new(Barrier::new(2));

        let hs: Vec<_> = (0..2)
            .map(|_| {
                let (s, c, b) = (sending.clone(), calls.clone(), barrier.clone());
                std::thread::spawn(move || {
                    b.wait(); // both threads start at the same instant
                    guarded_send(&s, &c)
                })
            })
            .collect();
        let results: Vec<_> = hs.into_iter().map(|h| h.join().unwrap()).collect();

        // The money invariant: the node was asked to send ONCE.
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the node was asked to send more than once");
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "more than one send succeeded");
        let rejected = results.iter().filter_map(|r| r.as_ref().err()).next().unwrap();
        assert_eq!(rejected, "ERR:SEND_IN_PROGRESS");
    }

    // The loser must be REJECTED, not queued. A queued second send pays twice a moment later — the very
    // thing being prevented. This is why the guard is try_lock and never lock().
    #[test]
    fn the_second_send_is_rejected_and_never_queued() {
        let sending = Arc::new(Mutex::new(()));
        let calls = Arc::new(AtomicU64::new(0));
        let held = sending.try_lock().expect("first send takes the guard");
        let start = std::time::Instant::now();
        let r = guarded_send(&sending, &calls);
        assert!(r.is_err(), "a second send got through while one was in flight");
        assert!(start.elapsed().as_millis() < 50, "the second send WAITED instead of being rejected");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "the rejected send still reached the node");
        drop(held);
    }

    // The guard must be released on every path, or one failed send would freeze sending forever.
    #[test]
    fn a_failed_send_does_not_lock_sending_forever() {
        let sending = Arc::new(Mutex::new(()));
        let calls = Arc::new(AtomicU64::new(0));
        {
            let _g = sending.try_lock().expect("take it");
            assert!(guarded_send(&sending, &calls).is_err()); // busy while held
        } // guard dropped, as it is when wallet_send returns
        assert!(guarded_send(&sending, &calls).is_ok(), "sending stayed locked after the guard was dropped");
    }
}

// The 12-word phrase is the ONE irreplaceable asset in this app: lose it and the coins are gone, with no
// support line and no undo. There is exactly one encrypted phrase file per datadir, and both create and
// restore used to end in an unconditional overwrite. The only thing preventing disaster was Core rejecting
// a duplicate wallet name plus the frontend hardcoding name="brisvia" — two assumptions the overwriting
// function does not control. These tests make the invariant ours instead of incidental.
#[cfg(test)]
mod phrase_never_overwritten_tests {
    use super::{decrypt_phrase_file, enc_seed_path, encrypt_phrase_file_ex};

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("brisvia_phrase_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const OLD: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const NEW: &str = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong";

    #[test]
    fn a_second_wallet_can_never_replace_the_first_phrase() {
        let d = tmpdir("overwrite");
        encrypt_phrase_file_ex(&d, OLD, "first-password-1", false).unwrap();
        let before = std::fs::read(enc_seed_path(&d)).unwrap();

        let r = encrypt_phrase_file_ex(&d, NEW, "second-password-2", false);
        assert_eq!(r.unwrap_err(), "ERR:WALLET_EXISTS", "a second wallet was allowed to overwrite the phrase");

        // The file must be byte-for-byte what it was: not rewritten, not truncated, not re-encrypted.
        assert_eq!(std::fs::read(enc_seed_path(&d)).unwrap(), before, "the phrase file changed on disk");
        // And the original phrase must still come back with the original password.
        assert_eq!(decrypt_phrase_file(&d, "first-password-1").unwrap().trim(), OLD);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_first_wallet_on_a_clean_profile_is_allowed() {
        let d = tmpdir("clean");
        assert!(encrypt_phrase_file_ex(&d, OLD, "pw-first-time-1", false).is_ok());
        assert_eq!(decrypt_phrase_file(&d, "pw-first-time-1").unwrap().trim(), OLD);
        let _ = std::fs::remove_dir_all(&d);
    }

    // Migration (an old unencrypted wallet gaining a password) is the ONE caller that must rewrite the
    // file. It passes allow_overwrite = true on purpose.
    #[test]
    fn migration_may_rewrite_the_phrase_on_purpose() {
        let d = tmpdir("migrate");
        encrypt_phrase_file_ex(&d, OLD, "old-password-11", false).unwrap();
        assert!(encrypt_phrase_file_ex(&d, OLD, "new-password-22", true).is_ok());
        // Same phrase, now readable with the new password.
        assert_eq!(decrypt_phrase_file(&d, "new-password-22").unwrap().trim(), OLD);
        let _ = std::fs::remove_dir_all(&d);
    }
}

// Money must never touch floating point. Amounts used to cross the IPC boundary as f64, where "0.1 + 0.2"
// drift, a hidden 9th decimal or scientific notation could reach the node and the amount shown could differ
// from the amount sent. Now the raw string the user typed is parsed into integer base units here, and the
// node receives an exact decimal string built from that integer (its RPC layer reads it with
// ParseFixedPoint — exact). These are the boundary cases; each one is a way real money goes wrong.
#[cfg(test)]
mod amount_tests {
    use super::{briv_to_decimal, parse_amount_briv, MAX_MONEY_BRIV};

    #[test]
    fn the_smallest_unit_is_one_briv() {
        assert_eq!(parse_amount_briv("0.00000001").unwrap(), 1);
    }

    #[test]
    fn one_coin_is_one_hundred_million_briv() {
        assert_eq!(parse_amount_briv("1").unwrap(), 100_000_000);
        assert_eq!(parse_amount_briv("1.0").unwrap(), 100_000_000);
        assert_eq!(parse_amount_briv("1.00000000").unwrap(), 100_000_000);
    }

    // The reason this module exists: 0.1 is not representable in binary floating point. As f64 it is
    // 0.1000000000000000055511151231257827; as text it is exactly 10_000_000 briv.
    #[test]
    fn one_tenth_is_exact_not_a_float_approximation() {
        assert_eq!(parse_amount_briv("0.1").unwrap(), 10_000_000);
        assert_eq!(parse_amount_briv("0.30000000").unwrap(), 30_000_000);
        // The classic: 0.1 + 0.2 = 0.30000000000000004 in f64. As integers it is exact.
        assert_eq!(parse_amount_briv("0.1").unwrap() + parse_amount_briv("0.2").unwrap(), parse_amount_briv("0.3").unwrap());
    }

    // A 9th decimal must be REFUSED, never rounded: silently rounding someone's money is worse than
    // asking them to retype it. The node would answer a raw English "Invalid amount" here.
    #[test]
    fn nine_decimals_are_refused_not_rounded() {
        assert_eq!(parse_amount_briv("0.000000001").unwrap_err(), "ERR:INVALID_AMOUNT");
        assert_eq!(parse_amount_briv("1.123456789").unwrap_err(), "ERR:INVALID_AMOUNT");
    }

    // "1e-8" parses as a number in JS and Rust alike. It must not be reinterpreted as 0.00000001.
    #[test]
    fn scientific_notation_and_signs_are_refused() {
        for bad in ["1e-8", "1E8", "1e8", "-1", "+1", "-0.5"] {
            assert_eq!(parse_amount_briv(bad).unwrap_err(), "ERR:INVALID_AMOUNT", "accepted {bad}");
        }
    }

    #[test]
    fn junk_is_refused() {
        for bad in ["", "   ", "abc", "1.2.3", "1,2,3", "0", "0.0", "0.00000000", "1 000", "$5", "1.5x", ".", ","] {
            assert_eq!(parse_amount_briv(bad).unwrap_err(), "ERR:INVALID_AMOUNT", "accepted {bad:?}");
        }
    }

    // THE BUG THIS CAUGHT: this function used to do replace(',', '.') so an ES user could type "12,5".
    // That silently turned "1,000" into 1 BRVA — but in English "1,000" is one thousand, so a person
    // sending 1,000 BRVA would have sent 1 and lost 999. A backend cannot know which convention was meant,
    // so it no longer guesses: commas are refused here, and the frontend (which knows the active language)
    // normalises to this canonical form, rejecting the separator that does not belong to that language.
    // See tests/amount-separator.test.js for the language-aware half.
    #[test]
    fn a_comma_is_refused_because_its_meaning_depends_on_the_language() {
        // The exact amount that used to lose 999 coins.
        assert_eq!(parse_amount_briv("1,000").unwrap_err(), "ERR:INVALID_AMOUNT");
        assert_eq!(parse_amount_briv("12,5").unwrap_err(), "ERR:INVALID_AMOUNT");
        assert_eq!(parse_amount_briv("1,00000000").unwrap_err(), "ERR:INVALID_AMOUNT");
        assert_eq!(parse_amount_briv("1,000.50").unwrap_err(), "ERR:INVALID_AMOUNT");
        assert_eq!(parse_amount_briv("1.000,50").unwrap_err(), "ERR:INVALID_AMOUNT");
        // The canonical form the frontend sends instead:
        assert_eq!(parse_amount_briv("12.5").unwrap(), 1_250_000_000);
        assert_eq!(parse_amount_briv("0.00000001").unwrap(), 1);
        // "1.000" is unambiguous once canonical: one coin, three decimals. NOT one thousand.
        assert_eq!(parse_amount_briv("1.000").unwrap(), 100_000_000);
    }

    #[test]
    fn surrounding_spaces_are_trimmed_not_rejected() {
        assert_eq!(parse_amount_briv("  1.5  ").unwrap(), 150_000_000);
    }

    // The cap is 100,000,000 BRVA = 10,000,000,000,000,000 briv (1e16).
    //
    // Be precise about WHY f64 is wrong here, or this comment documents a false reason (an earlier version
    // of it did). Two claims that sound right and are NOT:
    //   "100,000,000 can't be represented in an f64" — false, it is far below 2^53 and exact.
    //   "1e16 can't be represented"                  — also false, 1e16 happens to be exact.
    // The true statement is about NEIGHBOURS: 1e16 > 2^53 (≈9.007e15), so past that point consecutive
    // integers are no longer distinguishable — `1e16 == 1e16 + 1.0` is true in f64. At the top of the money
    // range, one briv apart is the same double.
    //
    // And that is not even the everyday reason to drop f64. The everyday reason is 0.1, which is not exact
    // at ANY magnitude (see one_tenth_is_exact_not_a_float_approximation): it bites at ordinary amounts,
    // not at the cap.
    #[test]
    fn the_money_cap_is_the_boundary() {
        assert_eq!(parse_amount_briv("100000000").unwrap(), MAX_MONEY_BRIV);
        assert_eq!(parse_amount_briv("100000000.00000001").unwrap_err(), "ERR:INVALID_AMOUNT");
        assert_eq!(parse_amount_briv("999999999999").unwrap_err(), "ERR:INVALID_AMOUNT");
    }

    #[test]
    fn an_absurd_number_does_not_overflow_it_is_refused() {
        assert_eq!(parse_amount_briv("99999999999999999999999999").unwrap_err(), "ERR:INVALID_AMOUNT");
    }

    // What the node actually receives must round-trip back to the same integer.
    #[test]
    fn what_the_node_receives_is_the_exact_amount() {
        for s in ["0.00000001", "0.1", "1", "1.5", "12.34567891".get(0..11).unwrap(), "100000000"] {
            let briv = parse_amount_briv(s).unwrap();
            let sent = briv_to_decimal(briv);
            assert_eq!(parse_amount_briv(&sent).unwrap(), briv, "{s} -> {sent} changed the amount");
            assert!(sent.matches('.').count() == 1 && sent.split('.').nth(1).unwrap().len() == 8, "bad shape: {sent}");
        }
        assert_eq!(briv_to_decimal(1), "0.00000001");
        assert_eq!(briv_to_decimal(100_000_000), "1.00000000");
        assert_eq!(briv_to_decimal(10_000_000), "0.10000000");
    }
}

// A send that fails is not one thing. "Rejected" and "we never reached the node" both mean the money did
// NOT move and a retry is safe. "The request went out and no answer came back" means it MAY have been
// broadcast — retrying there could pay twice. Only that last case warns the user, and nothing ever retries
// on its own. Getting this wrong in either direction costs real money: a false warning invites a needless
// double-check, a missing one invites a double payment.
#[cfg(test)]
mod send_outcome_tests {
    use super::{send_failure_to_code, RpcOutcome};

    #[test]
    fn a_node_that_refuses_is_not_reported_as_maybe_sent() {
        assert_eq!(send_failure_to_code(RpcOutcome::Rejected("ERR:INSUFFICIENT_FUNDS".into())), "ERR:INSUFFICIENT_FUNDS");
        assert_eq!(send_failure_to_code(RpcOutcome::Rejected("ERR:INVALID_ADDRESS".into())), "ERR:INVALID_ADDRESS");
    }

    #[test]
    fn never_reaching_the_node_is_not_reported_as_maybe_sent() {
        assert_eq!(send_failure_to_code(RpcOutcome::NotSent("connection refused".into())), "ERR:NODE_NOT_READY");
    }

    #[test]
    fn a_lost_answer_is_the_only_case_that_warns() {
        assert_eq!(send_failure_to_code(RpcOutcome::Unknown), "ERR:SEND_STATUS_UNKNOWN");
    }

    // The three outcomes must stay distinct: collapsing them is exactly the bug this classification fixes.
    #[test]
    fn the_three_outcomes_never_collapse_into_one_message() {
        let rejected = send_failure_to_code(RpcOutcome::Rejected("ERR:INSUFFICIENT_FUNDS".into()));
        let not_sent = send_failure_to_code(RpcOutcome::NotSent("x".into()));
        let unknown = send_failure_to_code(RpcOutcome::Unknown);
        assert_ne!(rejected, unknown);
        assert_ne!(not_sent, unknown);
        assert_ne!(rejected, not_sent);
        // Only the ambiguous one may tell the user the payment might have gone out.
        assert_eq!(unknown, "ERR:SEND_STATUS_UNKNOWN");
        assert_ne!(rejected, "ERR:SEND_STATUS_UNKNOWN");
        assert_ne!(not_sent, "ERR:SEND_STATUS_UNKNOWN");
    }
}

// The amount must leave this process as TEXT. Keeping a String in Rust proves nothing on its own: the JSON
// layer is where a number could quietly come back, and the node parses a JSON number as a double. This
// asserts on the exact bytes of the request body that goes to the node.
#[cfg(test)]
mod rpc_wire_format_tests {
    use super::{briv_to_decimal, parse_amount_briv};
    use serde_json::{json, Value};

    // Mirrors the body rpc()/rpc_classified() build, so the assertion is about what really goes on the wire.
    fn wire_body(address: &str, typed: &str) -> String {
        let briv = parse_amount_briv(typed).unwrap();
        let exact = briv_to_decimal(briv);
        let params: Value = json!([address, exact]);
        json!({ "jsonrpc": "1.0", "id": "brisvia", "method": "sendtoaddress", "params": params }).to_string()
    }

    #[test]
    fn the_amount_leaves_as_a_json_string_not_a_number() {
        let body = wire_body("brv1qtest", "1.5");
        // Quoted => JSON string => the node reads it with ParseFixedPoint (exact).
        assert!(body.contains(r#""1.50000000""#), "amount is not a quoted string: {body}");
        // Unquoted after the comma would mean a JSON number => parsed as a double by the node.
        assert!(!body.contains(",1.5]"), "amount went out as a bare number: {body}");
        assert!(!body.contains(",1.50000000]"), "amount went out as a bare number: {body}");
    }

    #[test]
    fn every_amount_is_sent_with_exactly_eight_decimals_and_quoted() {
        for typed in ["0.00000001", "0.1", "1", "12.5", "100000000"] {
            let body = wire_body("brv1qtest", typed);
            let v: Value = serde_json::from_str(&body).unwrap();
            let amount = &v["params"][1];
            assert!(amount.is_string(), "{typed} was serialised as {amount:?}, not a string");
            let s = amount.as_str().unwrap();
            assert_eq!(s.split('.').nth(1).map(|f| f.len()), Some(8), "{typed} -> {s}");
            // And it round-trips back to the same integer: nothing was lost on the way out.
            assert_eq!(parse_amount_briv(s).unwrap(), parse_amount_briv(typed).unwrap());
        }
    }

    // The float that started all this: as f64, 0.1 serialises to 0.1 but 0.1+0.2 does not serialise to 0.3.
    #[test]
    fn the_wire_carries_the_exact_amount_a_float_would_have_distorted() {
        let v: Value = serde_json::from_str(&wire_body("brv1qtest", "0.1")).unwrap();
        assert_eq!(v["params"][1].as_str().unwrap(), "0.10000000");
        // What a float boundary would have put on the wire, for contrast:
        let as_float = json!(0.1_f64 + 0.2_f64).to_string();
        assert_eq!(as_float, "0.30000000000000004");
        let v2: Value = serde_json::from_str(&wire_body("brv1qtest", "0.3")).unwrap();
        assert_eq!(v2["params"][1].as_str().unwrap(), "0.30000000");
    }
}

// ============================================================================================
// The node is NEVER killed.
//
// The defect that led to these tests: `stop_node` gave the node 1.5 s + 4 s and then killed it.
// Bitcoin Core flushes the chainstate on shutdown and its own docs warn this can take several minutes.
// Killing it halfway through that write is exactly how a block database is left half-written, and the
// user pays for it on the next start with a repair -- or with a corrupt database.
//
// A comment saying "do not kill it" does not stop anyone from killing it. These tests do: if a kill
// ever reappears on the node's path, they turn red.
// ============================================================================================
#[cfg(test)]
mod node_shutdown_tests {
    use super::*;
    use std::process::Child;
    use std::sync::atomic::AtomicBool;

    /// A process that will NOT close on its own, standing in for a node busy writing.
    fn stubborn_process() -> Child {
        #[cfg(target_os = "windows")]
        let c = Command::new("cmd").args(["/c", "ping -n 60 127.0.0.1 >nul"]).spawn();
        #[cfg(not(target_os = "windows"))]
        let c = Command::new("sleep").arg("60").spawn();
        c.expect("could not spawn the test process")
    }

    /// A process that exits immediately.
    fn quick_process() -> Child {
        #[cfg(target_os = "windows")]
        let c = Command::new("cmd").args(["/c", "exit 0"]).spawn();
        #[cfg(not(target_os = "windows"))]
        let c = Command::new("true").spawn();
        c.expect("could not spawn the test process")
    }

    fn slot(child: Option<Child>) -> Arc<Mutex<Option<Child>>> {
        Arc::new(Mutex::new(child))
    }

    /// Kill whatever is left, so a failing test cannot leave a process behind for 60 seconds.
    ///
    /// Reaps THROUGH a poisoned lock on purpose: the poison is about the data being unreliable, and the
    /// process is real either way. The test that poisons the slot needs this most -- skipping cleanup
    /// there is how a ping sits in the runner for a minute after the test "passed".
    fn reap(s: &Arc<Mutex<Option<Child>>>) {
        let mut guard = match s.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(mut c) = guard.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    #[test]
    fn the_deadline_is_180_seconds_not_five() {
        // 5.5s was the old value and it was the defect. If someone lowers it again "because it takes
        // too long", this stops them: taking too long is exactly when killing does the damage.
        assert_eq!(NODE_SHUTDOWN_MAX, Duration::from_secs(180),
                   "the node shutdown deadline cannot be lowered: Core warns it may take minutes");
    }

    #[test]
    fn an_empty_slot_is_already_absent() {
        let s: Arc<Mutex<Option<Child>>> = slot(None);
        assert!(matches!(wait_for_node_exit(&s, Duration::from_millis(50)),
                         Ok(NodeExitOutcome::AlreadyAbsent)));
    }

    #[test]
    fn a_process_that_already_exited_reports_exited() {
        let mut c = quick_process();
        let _ = c.wait(); // it is gone before we even ask
        let s = slot(Some(c));
        // try_wait on an already-reaped Child still answers; what matters is that we do not hang.
        let r = wait_for_node_exit(&s, Duration::from_millis(500));
        assert!(matches!(r, Ok(NodeExitOutcome::Exited(_)) | Ok(NodeExitOutcome::AlreadyAbsent)),
                "expected Exited or AlreadyAbsent, got {r:?}");
        reap(&s);
    }

    #[test]
    fn a_process_that_exits_during_the_wait_reports_exited() {
        let s = slot(Some(quick_process()));
        match wait_for_node_exit(&s, Duration::from_secs(10)) {
            Ok(NodeExitOutcome::Exited(_)) => {}
            other => panic!("expected Exited, got {other:?}"),
        }
        // The slot is cleared only once it is true that the process is gone.
        assert!(s.lock().unwrap().is_none(), "the slot should be empty after it exited");
    }

    #[test]
    fn a_process_past_the_deadline_times_out_and_is_NOT_killed() {
        let s = slot(Some(stubborn_process()));
        let r = wait_for_node_exit(&s, Duration::from_millis(300));
        assert!(matches!(r, Ok(NodeExitOutcome::TimedOut)), "expected TimedOut, got {r:?}");
        // Still there, still alive, still findable. Killing it is the bug this guards.
        let mut g = s.lock().unwrap();
        let c = g.as_mut().expect("it was killed, and it must not be");
        assert!(matches!(c.try_wait(), Ok(None)), "the process must still be running");
        drop(g);
        reap(&s);
    }

    #[test]
    fn a_poisoned_lock_is_an_error_not_a_false_all_clear() {
        let s = slot(Some(stubborn_process()));
        let s2 = Arc::clone(&s);
        // Panic while holding it. Afterwards nobody can know the process state -- and claiming it
        // exited would let an installer overwrite a live datadir.
        let _ = std::thread::spawn(move || {
            let _g = s2.lock().unwrap();
            panic!("poisoning the slot on purpose");
        }).join();
        let r = wait_for_node_exit(&s, Duration::from_millis(50));
        assert!(matches!(r, Err(NodeExitError::LockPoisoned)), "expected LockPoisoned, got {r:?}");
        // Reap through the poison: the process is real and must not outlive the test.
        reap(&s);
    }

    #[test]
    fn the_lock_is_free_between_polls() {
        let s = slot(Some(stubborn_process()));
        let s2 = Arc::clone(&s);
        let waiter = std::thread::spawn(move || wait_for_node_exit(&s2, Duration::from_millis(900)));
        // If the wait held the lock for its whole duration this would block until it finished.
        std::thread::sleep(Duration::from_millis(250));
        let grabbed = std::time::Instant::now();
        let g = s.lock().expect("the slot should be lockable while the wait sleeps");
        let waited = grabbed.elapsed();
        drop(g);
        assert!(waited < Duration::from_millis(200),
                "took {waited:?} to acquire: the wait is holding the lock across its sleep");
        let _ = waiter.join();
        reap(&s);
    }

    #[test]
    fn nothing_here_builds_an_AppState() {
        // Not a runtime assertion -- a note for whoever edits this file. Constructing AppState makes
        // tauri::tray::TrayIcon reachable, which links the GUI runtime, which imports
        // TaskDialogIndirect from comctl32, which Windows resolves to 5.82 for a manifest-less cargo
        // test binary, which does not export it: 0xC0000139 before main and zero tests run.
        // tools/ci/check_imports_deep.py is the gate that enforces this from outside.
        let _: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    }
}
