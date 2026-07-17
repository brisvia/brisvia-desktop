//! Stratum client for pool mining — the protocol layer of PoolWorkSource (Brisvia 1.0, "approach D").
//!
//! Per ChatGPT's SECURITY review (2026-07-11) the pool does ALL the sensitive work: it builds the coinbase,
//! chooses its own extranonce, computes the merkle root over the full transaction set, and serializes the
//! 80-byte header with the nonce set to zero. The miner receives that exact header and only rewrites the 4
//! nonce bytes (offset 76, little-endian) before hashing with RandomX. The submit carries ONLY {job_id, nonce}.
//! The pool re-validates from its own immutable job, so the miner can never divert the payout or forge a block.
//!
//! This module: connect, login, parse jobs (header_template), submit {job_id, nonce}, defensively.
//! - bounded message size (drop past MAX_MSG_BYTES); never executes anything from the server;
//! - the wallet NEVER leaves the miner — only the public payout address travels, in `login`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;
use serde_json::Value;

use crate::worksource::MiningJob;

/// Hard cap per line. A pool sending more than this on one line is misbehaving → drop it.
pub const MAX_MSG_BYTES: usize = 64 * 1024;

/// A pool job in "approach D": the header is already built by the pool; the miner only varies the nonce.
#[derive(Debug, Clone, PartialEq)]
pub struct PoolJob {
    pub job_id: String,
    pub header_template: String, // 160 hex chars = 80 bytes, nonce (bytes 76..80) set to zero
    pub nonce_offset: u32,       // where the nonce lives in the header (fixed at 76)
    pub seed_hash: String,       // RandomX key for this height (hex, as the pool uses it)
    pub share_target: String,    // per-miner target (hex, big-endian); the hash must be <= this
    pub clean_jobs: bool,        // true = drop previous jobs (they are stale)
    // Informational metadata (not used to build the header):
    pub height: u64,
    pub bits: String,
}

/// What the pool can send us.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Job(PoolJob),
    SetTarget(String),                                       // new share_target, no new job
    Ack { accepted: bool, stale: bool, reason: Option<String> }, // pool's verdict on our submit
    Block { accepted: bool },                               // our share was a block (accepted or lost the race)
    PoolError(String),                                      // pool-level error (bad login, too many bad shares…)
    LoginResult { ok: bool },
    Suspended { retry_after: Option<u64> },                 // the pool is under maintenance (explicit, not a crash)
    Other,                                                  // anything unrecognized — ignored, never fatal
}

fn get_str(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn get_u64(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

/// Decode a hex string to bytes. Returns None on odd length or a non-hex digit (never panics).
pub fn hex_to_bytes(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let b = s.as_bytes();
    let val = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut i = 0;
    while i < b.len() {
        out.push((val(b[i])? << 4) | val(b[i + 1])?);
        i += 2;
    }
    Some(out)
}

/// Parse one line from the pool into a typed message. NEVER panics; garbage/unknown → `Other`.
pub fn parse_incoming(line: &str) -> Incoming {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Incoming::Other,
    };
    match v.get("type").and_then(Value::as_str) {
        Some("job") => Incoming::Job(PoolJob {
            job_id: get_str(&v, "job_id"),
            header_template: get_str(&v, "header_template"),
            nonce_offset: v.get("nonce_offset").and_then(Value::as_u64).unwrap_or(76) as u32,
            seed_hash: get_str(&v, "seed_hash"),
            share_target: get_str(&v, "share_target"),
            clean_jobs: v.get("clean_jobs").and_then(Value::as_bool).unwrap_or(true),
            height: get_u64(&v, "height"),
            bits: get_str(&v, "bits"),
        }),
        Some("set_target") => Incoming::SetTarget(get_str(&v, "share_target")),
        Some("ack") => Incoming::Ack {
            accepted: v.get("accepted").and_then(Value::as_bool).unwrap_or(false),
            stale: v.get("stale").and_then(Value::as_bool).unwrap_or(false),
            reason: v.get("reason").filter(|r| !r.is_null()).map(|r| r.to_string()),
        },
        Some("block") => Incoming::Block {
            accepted: v.get("accepted").and_then(Value::as_bool).unwrap_or(false),
        },
        Some("error") => Incoming::PoolError(get_str(&v, "error")),
        // Explicit login answer. Every other message in this protocol is keyed by "type", but this case was
        // missing, so a pool answering {"type":"login_result","ok":...} fell through to Other and the miner
        // never learned whether it had been accepted.
        Some("login_result") => Incoming::LoginResult {
            ok: v.get("ok").and_then(Value::as_bool).unwrap_or(false),
        },
        // The pool is under maintenance. Distinct from a dropped socket so the miner can show "under
        // maintenance" and keep reconnecting instead of falling to solo or crying error. `retry_after_seconds`
        // (when present) lets the pool spread reconnections so every miner does not come back at once.
        Some("pool_suspended") => Incoming::Suspended {
            retry_after: v.get("retry_after_seconds").and_then(Value::as_u64),
        },
        _ => {
            // Legacy stratum-style ack (result/error with an id).
            if v.get("result").is_some() || v.get("id").is_some() {
                let ok = v.get("error").map(Value::is_null).unwrap_or(true);
                Incoming::LoginResult { ok }
            } else {
                Incoming::Other
            }
        }
    }
}

impl PoolJob {
    /// Convert a pool job into a MiningJob the RandomX engine can consume, or an error if any field is malformed.
    /// The header must be exactly 80 bytes; the nonce (offset 76..80) is left as-is (zero) for the engine to vary.
    pub fn to_mining_job(&self) -> Result<MiningJob, String> {
        let header80 = hex_to_bytes(&self.header_template).ok_or("bad header hex")?;
        if header80.len() != 80 {
            return Err(format!("header must be 80 bytes, got {}", header80.len()));
        }
        if self.nonce_offset != 76 {
            return Err(format!("unexpected nonce offset {}", self.nonce_offset));
        }
        let seed = hex_to_bytes(&self.seed_hash).ok_or("bad seed hex")?;
        if seed.len() != 32 {
            return Err(format!("seed must be 32 bytes, got {}", seed.len()));
        }
        let tgt = hex_to_bytes(&self.share_target).ok_or("bad target hex")?;
        if tgt.len() != 32 {
            return Err(format!("target must be 32 bytes, got {}", tgt.len()));
        }
        if tgt.iter().all(|&b| b == 0) {
            return Err("share target is zero".into()); // a zero target is unreachable → reject the job
        }
        let mut seed_key = [0u8; 32];
        seed_key.copy_from_slice(&seed);
        // seed_hash comes in display order (like getblocktemplate); RandomX uses the INTERNAL key
        // (reversed). Same as solo mode (mine_once) and the pool's verifier (rx_verifier).
        seed_key.reverse();
        let mut target_be = [0u8; 32];
        target_be.copy_from_slice(&tgt);
        Ok(MiningJob { header80, seed_key, target_be, job_id: self.job_id.clone(), height: self.height })
    }
}

/// Login message. ONLY the public payout address travels (never keys, seed, or wallet file).
pub fn login_msg(address: &str, worker: &str) -> String {
    serde_json::json!({ "method": "login", "params": { "address": address, "worker": worker } }).to_string()
}

/// Share submission (approach D): only the job id and the nonce. The pool holds everything else.
pub fn submit_msg(job_id: &str, nonce: u32) -> String {
    serde_json::json!({ "method": "submit", "params": { "job_id": job_id, "nonce": format!("{:08x}", nonce) } })
        .to_string()
}

/// Validate a user-supplied pool address before connecting ("custom pool" mode).
pub fn validate_pool_addr(host_port: &str) -> Result<(), String> {
    let hp = host_port.trim();
    if hp.chars().any(|c| c.is_control()) {
        return Err("invalid characters in address".into());
    }
    let (host, port) = hp.rsplit_once(':').ok_or_else(|| "use host:port".to_string())?;
    if host.is_empty() {
        return Err("empty host".into());
    }
    let port: u32 = port.parse().map_err(|_| "invalid port".to_string())?;
    if port == 0 || port > 65535 {
        return Err("port out of range".into());
    }
    // Block local targets by default (a pool at your own machine/LAN makes no sense for the official network;
    // a LAN pool would be an explicit advanced option). Hostnames pass; resolution happens at connect time.
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

/// Result of a bounded poll: a parsed message, a read timeout (no data yet), or a clean close.
#[derive(Debug, Clone, PartialEq)]
pub enum Poll {
    Message(Incoming),
    Timeout,
    Closed,
}

/// Why a login failed, which decides whether reconnecting makes any sense.
/// Permanent = the pool gave an answer that retrying cannot change (rejected address, banned worker, wrong
/// version). Reconnecting there hammers the pool forever and hides the real reason from the user.
/// Temporary = nobody answered (network, pool restarting): retrying is exactly right.
/// Suspended = the pool told us, explicitly, that it is under maintenance (`pool_suspended`). Like Temporary
/// we keep reconnecting with backoff and NEVER fall back to solo, but the UI shows "under maintenance" rather
/// than an error, so the user is not misled into thinking something broke.
#[derive(Debug, Clone, PartialEq)]
pub enum LoginError {
    Permanent(String),
    Temporary(String),
    Suspended { message: String, retry_after: Option<u64> },
}

impl LoginError {
    pub fn message(&self) -> &str {
        match self {
            LoginError::Permanent(m) | LoginError::Temporary(m) => m,
            LoginError::Suspended { message, .. } => message,
        }
    }
}

/// The connection under the protocol: plain TCP, or TCP wrapped in TLS.
///
/// Why TLS matters here: the miner's PAYOUT ADDRESS travels on this connection. Without encryption anyone in
/// the middle (a public wifi, a hostile network, an ISP) can rewrite it and collect the rewards of everyone
/// mining through them, or feed fake jobs so the CPU works for nothing. No private key ever crosses this
/// connection, so nothing can be stolen from the wallet -- but the earnings can be redirected, and the miner
/// would never notice.
///
/// The two variants exist because the official pool is reached by name over the internet (must be encrypted),
/// while tests talk to a loopback socket that has no certificate and never leaves the machine.
enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Conn {
    fn socket(&self) -> &TcpStream {
        match self {
            Conn::Plain(s) => s,
            Conn::Tls(s) => s.get_ref(),
        }
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

/// A minimal, defensive stratum connection. Protocol layer only; the miner loop drives the hashing.
///
/// Reader and writer share ONE connection (a TLS session cannot be cloned like a raw socket, and duplicating
/// it would corrupt the encrypted stream). BufReader owns it and writes go through `get_mut`.
pub struct StratumClient {
    io: BufReader<Conn>,
    line_buf: String, // persists a partial line across read timeouts (poll_message)
}

impl StratumClient {
    /// Plain TCP. For tests and for a user's custom pool that explicitly opts out of encryption.
    pub fn connect(host_port: &str, timeout: Duration) -> Result<Self, String> {
        // Note: `validate_pool_addr` is applied UPSTREAM (the backend validates a user's custom URL before it
        // ever reaches the worker). connect() itself must be able to reach any resolved address (the official
        // pool, and loopback in tests), so it does not re-validate here.
        let stream = Self::tcp(host_port, timeout)?;
        Ok(StratumClient { io: BufReader::new(Conn::Plain(stream)), line_buf: String::new() })
    }

    /// Encrypted. The certificate must be valid AND match `host_port`'s hostname: an impostor that redirects
    /// the traffic cannot present a certificate for pool.brisvia.com, so the connection fails instead of
    /// quietly mining for someone else. Used for the official pool.
    pub fn connect_tls(host_port: &str, timeout: Duration) -> Result<Self, String> {
        let host = host_port
            .rsplit_once(':')
            .map(|(h, _)| h)
            .ok_or_else(|| "use host:port".to_string())?
            .to_string();
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let cfg = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server = rustls::pki_types::ServerName::try_from(host.clone())
            .map_err(|_| format!("invalid pool hostname: {host}"))?;
        let conn = rustls::ClientConnection::new(std::sync::Arc::new(cfg), server)
            .map_err(|e| format!("TLS setup failed: {e}"))?;
        let stream = Self::tcp(host_port, timeout)?;
        let tls = rustls::StreamOwned::new(conn, stream);
        Ok(StratumClient { io: BufReader::new(Conn::Tls(Box::new(tls))), line_buf: String::new() })
    }

    fn tcp(host_port: &str, timeout: Duration) -> Result<TcpStream, String> {
        let addr = host_port
            .to_socket_addrs()
            .map_err(|e| format!("resolve failed: {e}"))?
            .next()
            .ok_or_else(|| "no address resolved".to_string())?;
        let stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("connect failed: {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(90))).ok();
        Ok(stream)
    }

    /// Read the next message but give up after `timeout`, returning `Poll::Timeout` so the caller can do other
    /// work (submit queued shares, check a stop flag) and poll again. A partial line is kept across timeouts.
    pub fn poll_message(&mut self, timeout: Duration) -> Result<Poll, String> {
        self.io.get_ref().socket().set_read_timeout(Some(timeout)).ok();
        match self.io.read_line(&mut self.line_buf) {
            Ok(0) => Ok(Poll::Closed),
            Ok(_) => {
                let line = std::mem::take(&mut self.line_buf);
                if line.len() >= MAX_MSG_BYTES {
                    return Err("message too large".into());
                }
                Ok(Poll::Message(parse_incoming(line.trim_end())))
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if self.line_buf.len() >= MAX_MSG_BYTES {
                    return Err("message too large".into());
                }
                Ok(Poll::Timeout)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Shut down the connection (unblocks a reader parked in poll/next_message).
    pub fn shutdown(&self) {
        let _ = self.io.get_ref().socket().shutdown(std::net::Shutdown::Both);
    }

    fn send_line(&mut self, line: &str) -> Result<(), String> {
        // Writes go through the same connection the reader owns: a TLS session cannot be split in two.
        let w = self.io.get_mut();
        w.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        w.write_all(b"\n").map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())
    }

    /// Log in and WAIT for the pool's answer before mining a single hash.
    ///
    /// Sending the login and starting to mine straight away (what this did before) means a miner whose address
    /// the pool rejects burns its CPU for nothing: the pool credits no one, and the user is never told. So we
    /// block here until the pool speaks.
    ///
    /// A pool that sends work IS an acceptance: several pools skip the explicit ok and just start sending jobs.
    /// That first job is not thrown away — it is handed back so the session mines it.
    ///
    /// Silence is NOT a failure. Brisvia's own pool answers a good login with work, and sends nothing at all
    /// while it has no template ready (starting up, or between blocks). Treating that as an error would drop a
    /// perfectly good connection and reconnect in a loop. So the timeout means "accepted, no work yet": the
    /// session goes on and waits for a job. Nothing is mined in the meantime — without a job there is nothing
    /// to hash — so waiting is safe, and honest about what is happening.
    ///
    /// The error distinguishes PERMANENT from temporary, because the caller must not reconnect in a loop against
    /// a pool that already said no.
    pub fn login(&mut self, address: &str, worker: &str, timeout: Duration) -> Result<Option<PoolJob>, LoginError> {
        self.send_line(&login_msg(address, worker))
            .map_err(LoginError::Temporary)?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Ok(None); // accepted, no work yet — wait for a job instead of reconnecting
            }
            match self.poll_message(left.min(Duration::from_millis(500))) {
                Ok(Poll::Message(Incoming::LoginResult { ok: true })) => return Ok(None),
                // Rejected: the address is wrong, the worker is banned, the version does not match. Retrying
                // cannot change the answer, so this must never become a reconnect loop.
                Ok(Poll::Message(Incoming::LoginResult { ok: false })) => {
                    return Err(LoginError::Permanent("the pool rejected the login".into()))
                }
                Ok(Poll::Message(Incoming::PoolError(e))) => return Err(LoginError::Permanent(e)),
                // The pool is under maintenance: not an error and not a reason to go solo. Reconnect after the
                // suggested delay and let the UI say so.
                Ok(Poll::Message(Incoming::Suspended { retry_after })) => {
                    return Err(LoginError::Suspended {
                        message: "the pool is under maintenance".into(),
                        retry_after,
                    })
                }
                // Work arrived without an explicit ok: that IS the acceptance. Keep the job.
                Ok(Poll::Message(Incoming::Job(j))) => return Ok(Some(j)),
                Ok(Poll::Message(_)) | Ok(Poll::Timeout) => continue, // anything else before the verdict: ignore
                Ok(Poll::Closed) => {
                    return Err(LoginError::Temporary("the pool closed the connection".into()))
                }
                Err(e) => return Err(LoginError::Temporary(e)),
            }
        }
    }

    pub fn submit(&mut self, job_id: &str, nonce: u32) -> Result<(), String> {
        self.send_line(&submit_msg(job_id, nonce))
    }

    /// Read the next message, bounded by MAX_MSG_BYTES. `Ok(None)` on a clean close.
    pub fn next_message(&mut self) -> Result<Option<Incoming>, String> {
        let mut line = String::new();
        let n = (&mut self.io)
            .by_ref()
            .take(MAX_MSG_BYTES as u64)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        if line.len() >= MAX_MSG_BYTES {
            return Err("message too large".into());
        }
        Ok(Some(parse_incoming(line.trim_end())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_job_line() -> String {
        // header_template: 80 bytes hex (nonce zeroed), seed/target 32 bytes each.
        let header = "0".repeat(160);
        let seed = "54".repeat(32);
        let target = "0f".to_string() + &"ff".repeat(31);
        format!(
            r#"{{"type":"job","job_id":"a1","header_template":"{header}","nonce_offset":76,"seed_hash":"{seed}","share_target":"{target}","clean_jobs":true,"height":3000,"bits":"1e7fffff"}}"#
        )
    }

    #[test]
    fn parses_a_job_and_converts() {
        match parse_incoming(&sample_job_line()) {
            Incoming::Job(j) => {
                assert_eq!(j.job_id, "a1");
                assert_eq!(j.nonce_offset, 76);
                assert_eq!(j.height, 3000);
                let mj = j.to_mining_job().expect("valid job");
                assert_eq!(mj.header80.len(), 80);
                assert_eq!(mj.seed_key, [0x54u8; 32]);
                assert_eq!(mj.job_id, "a1");
            }
            other => panic!("expected Job, got {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_header_length() {
        let j = PoolJob {
            job_id: "x".into(),
            header_template: "00ff".into(), // 2 bytes, not 80
            nonce_offset: 76,
            seed_hash: "54".repeat(32),
            share_target: "ff".repeat(32),
            clean_jobs: true,
            height: 1,
            bits: "1e7fffff".into(),
        };
        assert!(j.to_mining_job().is_err());
    }

    #[test]
    fn parses_set_target_and_garbage() {
        assert_eq!(parse_incoming(r#"{"type":"set_target","share_target":"1f"}"#), Incoming::SetTarget("1f".into()));
        assert_eq!(parse_incoming("not json at all"), Incoming::Other);
        assert_eq!(parse_incoming("{}"), Incoming::Other);
    }

    #[test]
    fn parses_pool_suspended_with_and_without_retry() {
        assert_eq!(
            parse_incoming(r#"{"type":"pool_suspended","retry_after_seconds":45}"#),
            Incoming::Suspended { retry_after: Some(45) }
        );
        // A pool that omits the hint is still valid maintenance; the miner falls back to its own 30–60s window.
        assert_eq!(
            parse_incoming(r#"{"type":"pool_suspended"}"#),
            Incoming::Suspended { retry_after: None }
        );
    }

    #[test]
    fn submit_is_only_jobid_and_nonce() {
        let msg = submit_msg("a1", 0x0000_2a3c);
        assert!(msg.contains("00002a3c") && msg.contains("a1"));
        // approach D: no extranonce2/ntime/header from the miner
        assert!(!msg.contains("extranonce") && !msg.contains("ntime") && !msg.contains("header"));
    }

    #[test]
    fn login_never_leaks_more_than_address() {
        let msg = login_msg("brv1qexample", "rig1");
        assert!(msg.contains("brv1qexample") && msg.contains("rig1"));
        assert!(!msg.contains("seed") && !msg.contains("priv") && !msg.contains("xprv"));
    }

    #[test]
    fn hex_decoding() {
        assert_eq!(hex_to_bytes("00ff").unwrap(), vec![0x00, 0xff]);
        assert!(hex_to_bytes("zz").is_none());
        assert!(hex_to_bytes("abc").is_none()); // odd length
    }

    #[test]
    fn addr_validation() {
        assert!(validate_pool_addr("pool.brisvia.com:3333").is_ok());
        assert!(validate_pool_addr("203.0.113.5:3333").is_ok()); // public IP
        assert!(validate_pool_addr("noport").is_err());
        assert!(validate_pool_addr("host:0").is_err());
        assert!(validate_pool_addr("host:99999").is_err());
        // local/private targets are blocked by default
        assert!(validate_pool_addr("localhost:3333").is_err());
        assert!(validate_pool_addr("127.0.0.1:3333").is_err());
        assert!(validate_pool_addr("192.168.1.10:3333").is_err());
        assert!(validate_pool_addr("10.0.0.5:3333").is_err());
        assert!(validate_pool_addr("pool.brisvia.com:33\n33").is_err()); // control chars
    }

    #[test]
    fn rejects_zero_target() {
        let j = PoolJob {
            job_id: "x".into(),
            header_template: "0".repeat(160),
            nonce_offset: 76,
            seed_hash: "54".repeat(32),
            share_target: "00".repeat(32), // zero target is unreachable
            clean_jobs: true,
            height: 1,
            bits: "1e7fffff".into(),
        };
        assert!(j.to_mining_job().is_err());
    }
}
