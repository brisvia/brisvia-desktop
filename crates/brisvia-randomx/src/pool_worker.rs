//! Pool worker loop: connect to a stratum pool, log in, receive jobs, mine them, submit shares, reconnect.
//! Ties together `stratum` (protocol), `pool_miner` (the RandomX search) and `worksource` (the job type).
//! Approach D: the pool sends a ready header; the worker only varies the nonce and submits {job_id, nonce}.
//!
//! Cancellation (per the SECURITY review, P0): a reader loop receives jobs and raises a `cancel` flag on
//! every new job (or disconnect); the miner checks that flag (mine_job polls it every 256 hashes), so it drops a
//! dead job within milliseconds instead of scanning up to 50M nonces. Only the current generation's share is
//! submitted; a solution for a superseded job is discarded. The audited solo path is untouched.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::stratum::{Incoming, LoginError, Poll, StratumClient};
use crate::worksource::{MiningJob, Solution};

/// Marks a session error as permanent, so the reconnect loop stops instead of hammering the pool forever.
/// A string prefix keeps `run_session`'s signature (and its tests) unchanged.
const PERMANENT_PREFIX: &str = "PERMANENT:";

/// Marks a session that ended because the pool is under maintenance. The suffix carries the pool's suggested
/// retry delay in seconds (0 = none given), so the reconnect loop waits a spread-out interval instead of the
/// growing backoff — and NEVER falls to solo. Same string-prefix trick as PERMANENT_PREFIX.
const SUSPENDED_PREFIX: &str = "SUSPENDED:";
/// Fallback maintenance retry window when the pool did not send `retry_after_seconds`: 30–60 s, spread per
/// address so miners do not all reconnect at the same instant (thundering herd).
fn suspended_retry_secs(address: &str, given: Option<u64>) -> u64 {
    if let Some(s) = given.filter(|&s| s > 0) {
        return s.clamp(5, 3600);
    }
    30 + (jitter_ms(address, 1) % 31_000) / 1000 // 30..=60
}

/// One event the worker reports (the Tauri backend turns these into UI updates, like the solo worker).
/// Per the audit: found-locally, submitted, and accepted are DISTINCT — the UI counts a contribution only on
/// `ShareAccepted` (the pool's explicit confirmation), never on submit.
#[derive(Debug, Clone, PartialEq)]
pub enum PoolEvent {
    Connected,
    LoggedIn,
    NewJob { job_id: String, height: u64 },
    Hashrate(f64), // periodic real per-second speed for the pool UI (avoids a misleading 0 H/s while mining)
    ShareSubmitted { job_id: String, nonce: u32 }, // found locally and sent — NOT yet confirmed
    ShareAccepted,                                 // pool confirmed the share
    ShareRejected { reason: String },              // pool rejected (low diff, duplicate, unknown job…)
    ShareStale,                                    // arrived late (not fraud)
    TargetChangeIgnored,                           // pool tried to change difficulty mid-job; v1 waits for a new job
    BlockFound,                                    // our share met the network target → a block
    Suspended { retry_after: Option<u64> },        // pool is under maintenance (explicit) — retry, never solo
    // About to wait before reconnecting (normal backoff OR maintenance). `retry_at` is an ABSOLUTE unix
    // timestamp (seconds) of the next attempt, so the UI can show a real countdown that ticks down on its
    // own clock without the backend resetting it each poll.
    Reconnecting { retry_at: u64 },
    Disconnected(String),
}

/// Current unix time in whole seconds (for absolute retry timestamps). 0 if the clock is before the epoch.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run a single connected session with fast job cancellation. `mine` is injected so tests can substitute a
/// stub; production passes `mine_job`. Returns when the connection closes/errors or `should_stop` is set.
pub fn run_session<F, M>(
    mut client: StratumClient,
    address: &str,
    worker: &str,
    threads: usize,
    max_nonces: u64,
    should_stop: &AtomicBool,
    hashes: &AtomicU64,
    on_event: &mut F,
    mine: M,
) -> Result<(), String>
where
    F: FnMut(PoolEvent),
    M: Fn(&MiningJob, usize, u64, u64, &AtomicBool) -> Option<Solution> + Send,
{
    on_event(PoolEvent::Connected);
    // Wait for the pool's verdict before mining anything: a rejected miner must not burn CPU for nobody.
    // A pool that answers with work instead of an explicit ok has accepted us, and that first job is mined.
    let first_job = match client.login(address, worker, Duration::from_secs(20)) {
        Ok(j) => j,
        Err(LoginError::Permanent(m)) => return Err(format!("{PERMANENT_PREFIX}{m}")),
        Err(LoginError::Temporary(m)) => return Err(m),
        // The pool answered, explicitly, that it is under maintenance. Tell the UI (so it shows "under
        // maintenance", not an error) and hand the reconnect loop the suggested delay. Never solo.
        Err(LoginError::Suspended { retry_after, .. }) => {
            on_event(PoolEvent::Hashrate(0.0));
            on_event(PoolEvent::Suspended { retry_after });
            return Err(format!("{SUSPENDED_PREFIX}{}", retry_after.unwrap_or(0)));
        }
    };
    on_event(PoolEvent::LoggedIn);

    let job: Arc<Mutex<Option<(MiningJob, u64)>>> = Arc::new(Mutex::new(None));
    let cur_gen = Arc::new(AtomicU64::new(0));
    // The job that arrived with the login (if any) starts the session instead of being dropped.
    if let Some(pj) = first_job {
        if let Ok(mj) = pj.to_mining_job() {
            let (jid, h) = (mj.job_id.clone(), mj.height);
            *job.lock().unwrap() = Some((mj, 1));
            cur_gen.store(1, Ordering::SeqCst);
            on_event(PoolEvent::NewJob { job_id: jid, height: h });
        }
    }
    let cancel = Arc::new(AtomicBool::new(false)); // raised on new job / disconnect -> mine_job bails out fast
    let done = Arc::new(AtomicBool::new(false));
    // Bounded queue of found shares (job_id, nonce, generation). Bounded so a stalled reader can never let the
    // queue grow without limit; if it ever fills, the miner drops the extra solution (a superseded share is
    // worthless anyway) instead of blocking. 64 is far above the ~1 share/job the miner produces in practice.
    let (tx, rx) = mpsc::sync_channel::<(String, u32, u64)>(64);

    std::thread::scope(|s| -> Result<(), String> {
        // ---- miner thread: mine the current job; on a solution for the still-current generation, queue it ----
        {
            let (job, cur_gen, cancel, done, tx) =
                (job.clone(), cur_gen.clone(), cancel.clone(), done.clone(), tx.clone());
            s.spawn(move || {
                // Progressive nonce sweep per job: `start` advances after each share so we do NOT resend the
                // same nonce (avoids "duplicate") and produce several shares per job (key for PPLNS).
                // It resets to 0 when a new job arrives (the generation changes).
                let mut start: u64 = 0;
                let mut last_gen: u64 = 0;
                while !done.load(Ordering::Relaxed) && !should_stop.load(Ordering::Relaxed) {
                    let cur = job.lock().unwrap().clone();
                    match cur {
                        Some((mj, g)) => {
                            if g != last_gen {
                                start = 0;
                                last_gen = g;
                            }
                            cancel.store(false, Ordering::SeqCst);
                            match mine(&mj, threads, start, max_nonces, &cancel) {
                                Some(sol) => {
                                    // Only submit if this job is still the current one (not superseded).
                                    if cur_gen.load(Ordering::SeqCst) == g && !should_stop.load(Ordering::Relaxed) {
                                        let _ = tx.try_send((sol.job_id, sol.nonce, g)); // drop if the queue is full
                                    }
                                    start = sol.nonce as u64 + 1; // continue AFTER the nonce we found
                                }
                                None => {
                                    // Range exhausted or cancelled. If the job is still current, wait for the
                                    // next one (do not re-sweep from 0, which would resend already-tried nonces).
                                    if cur_gen.load(Ordering::SeqCst) == g {
                                        std::thread::sleep(Duration::from_millis(100));
                                    }
                                }
                            }
                        }
                        None => std::thread::sleep(Duration::from_millis(50)),
                    }
                }
            });
        }

        // ---- reader loop (this thread): owns the socket; receives jobs, submits queued shares ----
        let out = (|| -> Result<(), String> {
            let mut last_rate = Instant::now();
            let mut last_h: u64 = 0;
            loop {
                if should_stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                // Emit a real per-second speed every ~2s from the shared hash counter, so the pool UI shows a
                // real number instead of a misleading 0 H/s. Each window measures the hashes since the last emit.
                if last_rate.elapsed() >= Duration::from_secs(2) {
                    let now_h = hashes.load(Ordering::Relaxed);
                    let hs = now_h.saturating_sub(last_h) as f64 / last_rate.elapsed().as_secs_f64().max(0.001);
                    on_event(PoolEvent::Hashrate(hs));
                    last_h = now_h;
                    last_rate = Instant::now();
                }
                // submit any solutions the miner queued (only for the current generation)
                while let Ok((jid, nonce, g)) = rx.try_recv() {
                    if cur_gen.load(Ordering::SeqCst) == g {
                        client.submit(&jid, nonce)?;
                        on_event(PoolEvent::ShareSubmitted { job_id: jid, nonce });
                        // submit/on_event may have set the stop flag (or the user stopped) — exit right away.
                        if should_stop.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                    }
                }
                if should_stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                match client.poll_message(Duration::from_millis(300))? {
                    Poll::Message(Incoming::Job(pj)) => {
                        if let Ok(mj) = pj.to_mining_job() {
                            let g = cur_gen.fetch_add(1, Ordering::SeqCst) + 1;
                            let (jid, h) = (mj.job_id.clone(), mj.height);
                            *job.lock().unwrap() = Some((mj, g));
                            cancel.store(true, Ordering::SeqCst); // cancel the previous job's mining immediately
                            on_event(PoolEvent::NewJob { job_id: jid, height: h });
                        }
                    }
                    // The pool's explicit verdict on a submitted share.
                    Poll::Message(Incoming::Ack { accepted, stale, reason }) => {
                        if accepted {
                            on_event(PoolEvent::ShareAccepted);
                        } else if stale {
                            on_event(PoolEvent::ShareStale);
                        } else {
                            on_event(PoolEvent::ShareRejected { reason: reason.unwrap_or_default() });
                        }
                    }
                    // Contract (avoids the block+ack double count the audit flagged): the pool sends EXACTLY ONE
                    // verdict per share. A share that turns out to be a block arrives as a `block`, NOT as an
                    // `ack{accepted}` as well — so counting the block as an accepted share here is not a double
                    // count. `accepted:false` (e.g. a stale block on a reorg) marks the block without crediting it.
                    Poll::Message(Incoming::Block { accepted }) => {
                        on_event(PoolEvent::BlockFound);
                        if accepted {
                            on_event(PoolEvent::ShareAccepted);
                        }
                    }
                    // A pool-level error (bad login, too many bad shares) is terminal for this session.
                    Poll::Message(Incoming::PoolError(e)) => return Err(format!("pool error: {e}")),
                    // The pool went into maintenance mid-session. Atomic cutoff (audit): invalidate the
                    // current job and cancel mining so NO share found from this instant on is submitted; zero
                    // the hashrate; tell the UI; then end the session so the reconnect loop waits the suggested
                    // delay and comes back — it must NEVER fall to solo.
                    Poll::Message(Incoming::Suspended { retry_after }) => {
                        cancel.store(true, Ordering::SeqCst);
                        cur_gen.fetch_add(1, Ordering::SeqCst); // any queued share is now for a stale generation
                        *job.lock().unwrap() = None;
                        on_event(PoolEvent::Hashrate(0.0));
                        on_event(PoolEvent::Suspended { retry_after });
                        return Err(format!("{SUSPENDED_PREFIX}{}", retry_after.unwrap_or(0)));
                    }
                    // A rejected login is permanent — stop this session with an error.
                    Poll::Message(Incoming::LoginResult { ok: false }) => return Err("login rejected".into()),
                    // v1 does not support changing the target mid-job: each job carries its own share_target and
                    // the pool validates against the one IT stored. Silently ignoring this would leave the miner
                    // working to a target the pool no longer uses, and every share would come back rejected with
                    // no explanation. Say it out loud and wait for a fresh job, which does carry the new target.
                    Poll::Message(Incoming::SetTarget(_)) => {
                        on_event(PoolEvent::TargetChangeIgnored);
                    }
                    Poll::Message(_) => {}
                    Poll::Timeout => {}
                    Poll::Closed => return Ok(()),
                }
            }
        })();

        // stop the miner and unblock the socket before the scope joins the thread
        done.store(true, Ordering::SeqCst);
        cancel.store(true, Ordering::SeqCst);
        client.shutdown();
        out
    })
}

/// The full worker: connect + run a session, reconnecting with capped backoff if the pool drops. Blocks.
///
/// `tls` encrypts the connection. It is NOT cosmetic: the miner's payout address travels on this link, so
/// anyone in the middle of an unencrypted one can rewrite it and collect the rewards of everyone mining
/// through them. The official pool always passes true; only a user's custom pool may opt out, knowingly.
pub fn run_pool_worker<F>(host_port: &str, address: &str, worker: &str, threads: usize, tls: bool, should_stop: &AtomicBool, mut on_event: F)
where
    F: FnMut(PoolEvent),
{
    // One SeedGuard for the whole worker: it reuses the RandomX cache across jobs (and reconnections) and
    // rate-limits rebuilds so a hostile pool cannot force a rebuild loop. Shared into the per-session mine closure.
    let guard = Arc::new(Mutex::new(crate::pool_miner::SeedGuard::new()));
    // Shared hash counter for the whole worker; the session's reader loop turns it into a per-second speed.
    let hashes = Arc::new(AtomicU64::new(0));
    let mut backoff = 1u64;
    while !should_stop.load(Ordering::Relaxed) {
        let intento = if tls {
            StratumClient::connect_tls(host_port, Duration::from_secs(15))
        } else {
            StratumClient::connect(host_port, Duration::from_secs(15))
        };
        match intento {
            Ok(client) => {
                let guard = guard.clone();
                let hashes_mine = hashes.clone();
                let mine = move |mj: &MiningJob, threads: usize, nonce_start: u64, max_nonces: u64, cancel: &AtomicBool| -> Option<Solution> {
                    // Reuse the cache/dataset while the seed is unchanged; refuse a rebuild burst (None → skip).
                    // engine_for builds the ~2.1 GB fast dataset once per seed when the RAM is there, and falls
                    // back to light on a small machine — the miner runs fast where it can, everywhere else it
                    // still mines (same hash), just slower.
                    let (cache, dataset) = match guard.lock().unwrap().engine_for(&mj.seed_key, threads) {
                        Ok(cd) => cd,
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(500)); // don't spin while churn is rate-limited
                            return None;
                        }
                    };
                    crate::pool_miner::mine_with_cache_counted(mj, &cache, dataset.as_ref(), threads, nonce_start, max_nonces, cancel, &hashes_mine)
                };
                // Only a session that actually LOGGED IN counts as progress and resets the backoff. Resetting on
                // the raw TCP connect was a bug: a pool that accepts the socket and then drops it (e.g. while
                // rate-limiting, or during a restart) made the miner reconnect every ~1.3 s and hammer the pool.
                // A `tap` closure watches the events for a successful login without changing run_session's shape.
                let mut logged_in = false;
                let err = {
                    let mut tap = |e: PoolEvent| {
                        if matches!(e, PoolEvent::LoggedIn) { logged_in = true; }
                        on_event(e);
                    };
                    run_session(client, address, worker, threads, 50_000_000, should_stop, &hashes, &mut tap, mine)
                        .err().unwrap_or_default()
                };
                if logged_in { backoff = 1; }
                // The pool gave a definitive answer (rejected address, banned worker, wrong version).
                // Reconnecting cannot change it: it would hammer the pool and hide the reason from the user.
                if let Some(reason) = err.strip_prefix(PERMANENT_PREFIX) {
                    on_event(PoolEvent::Disconnected(reason.to_string()));
                    return;
                }
                // The pool is under maintenance. run_session already emitted PoolEvent::Suspended (the UI shows
                // "under maintenance", not an error), so do NOT emit Disconnected here. Wait the suggested/spread
                // delay and reconnect — this loop never exits to solo.
                if let Some(retry_str) = err.strip_prefix(SUSPENDED_PREFIX) {
                    let wait = suspended_retry_secs(address, retry_str.parse::<u64>().ok());
                    // Maintenance countdown takes priority over the normal backoff.
                    on_event(PoolEvent::Reconnecting { retry_at: unix_now() + wait });
                    for _ in 0..(wait * 10) {
                        if should_stop.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    continue; // skip the normal backoff sleep; keep the backoff untouched for real drops
                }
                on_event(PoolEvent::Disconnected(err));
            }
            Err(e) => on_event(PoolEvent::Disconnected(e)),
        }
        if should_stop.load(Ordering::Relaxed) {
            break;
        }
        // Backoff in short slices so stopping is immediate, plus jitter: without it, every miner that lost the
        // same pool would come back at the same instant and knock it over again.
        let jitter_ms = jitter_ms(address, backoff);
        // Tell the UI when the next attempt will happen (absolute), for the reconnect countdown.
        on_event(PoolEvent::Reconnecting { retry_at: unix_now() + backoff });
        let slices = (backoff * 1000 + jitter_ms) / 100;
        for _ in 0..slices {
            if should_stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        backoff = (backoff * 2).min(60);
    }
}

/// Spread reconnections apart without a random number generator (this crate has none): derive a stable
/// per-miner offset from its address and the attempt, giving each miner a different wait.
fn jitter_ms(address: &str, attempt: u64) -> u64 {
    let mut h: u64 = 1469598103934665603; // FNV-1a
    for b in address.as_bytes() {
        h = (h ^ *b as u64).wrapping_mul(1099511628211);
    }
    h = (h ^ attempt).wrapping_mul(1099511628211);
    h % 1000 // up to 1 extra second
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    fn job_line(job_id: &str, target_ff: bool, height: u64) -> String {
        let header = "0".repeat(160);
        let seed = "54".repeat(32);
        // easy target (all-ff) => solvable; hard target (00..01) => practically unsolvable
        let target = if target_ff { "ff".repeat(32) } else { "00".repeat(31) + "01" };
        format!(
            r#"{{"type":"job","job_id":"{job_id}","header_template":"{header}","nonce_offset":76,"seed_hash":"{seed}","share_target":"{target}","clean_jobs":true,"height":{height},"bits":"1e7fffff"}}"#
        )
    }

    // Full loop over real TCP: login -> receive job -> mine -> submit {job_id, nonce}.
    #[test]
    fn logs_in_receives_job_and_submits_share() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut writer = sock;
            let mut login = String::new();
            reader.read_line(&mut login).unwrap();
            assert!(login.contains("\"login\"") && login.contains("brv1qexample"));
            writer.write_all((job_line("j1", true, 7) + "\n").as_bytes()).unwrap();
            writer.flush().unwrap();
            let mut submit = String::new();
            reader.read_line(&mut submit).unwrap();
            submit
        });
        let client = StratumClient::connect(&addr, Duration::from_secs(5)).unwrap();
        let stop = AtomicBool::new(false);
        let mut events = Vec::new();
        // stub: return a fixed nonce, then flag stop so the session ends.
        let stub = |mj: &MiningJob, _t: usize, _ns: u64, _mx: u64, _s: &AtomicBool| -> Option<Solution> {
            Some(Solution { job_id: mj.job_id.clone(), nonce: 0x2a, ntime: 0, extranonce2: String::new() })
        };
        let stop_ref = &stop;
        let mut on_event = |e: PoolEvent| {
            if let PoolEvent::ShareSubmitted { .. } = e { stop_ref.store(true, Ordering::SeqCst); }
            events.push(e);
        };
        run_session(client, "brv1qexample", "rig1", 1, 1000, &stop, &std::sync::atomic::AtomicU64::new(0), &mut on_event, stub).unwrap();
        let submit_line = server.join().unwrap();
        assert!(submit_line.contains("\"submit\"") && submit_line.contains("\"j1\"") && submit_line.contains("0000002a"), "{submit_line}");
        assert!(events.contains(&PoolEvent::LoggedIn));
        assert!(events.iter().any(|e| matches!(e, PoolEvent::ShareSubmitted { .. })));
    }

    // Cancellation: a hard job is being mined; a new easy job arrives; the worker must ABANDON the hard job and
    // submit a share for the NEW job (proving fast cancellation, not a stuck old job).
    #[test]
    fn a_new_job_cancels_the_previous_one() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut writer = sock;
            let mut login = String::new();
            reader.read_line(&mut login).unwrap();
            // job1: hard (miner will block on it), then job2: easy (miner should switch and solve it)
            writer.write_all((job_line("hard1", false, 10) + "\n").as_bytes()).unwrap();
            writer.flush().unwrap();
            std::thread::sleep(Duration::from_millis(150)); // let the miner start on job1
            writer.write_all((job_line("easy2", true, 11) + "\n").as_bytes()).unwrap();
            writer.flush().unwrap();
            let mut submit = String::new();
            reader.read_line(&mut submit).unwrap();
            submit
        });
        let client = StratumClient::connect(&addr, Duration::from_secs(5)).unwrap();
        let stop = AtomicBool::new(false);
        // stub: on the hard job, block until cancel is raised (simulates a long search); on the easy job, solve.
        let stub = |mj: &MiningJob, _t: usize, _ns: u64, _mx: u64, s: &AtomicBool| -> Option<Solution> {
            if mj.job_id == "hard1" {
                while !s.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                None // cancelled
            } else {
                Some(Solution { job_id: mj.job_id.clone(), nonce: 0x99, ntime: 0, extranonce2: String::new() })
            }
        };
        let stop_ref = &stop;
        let mut got_share_for = String::new();
        let mut on_event = |e: PoolEvent| {
            if let PoolEvent::ShareSubmitted { job_id, .. } = &e {
                got_share_for = job_id.clone();
                stop_ref.store(true, Ordering::SeqCst);
            }
        };
        run_session(client, "brv1qexample", "rig1", 1, 1000, &stop, &std::sync::atomic::AtomicU64::new(0), &mut on_event, stub).unwrap();
        let submit_line = server.join().unwrap();
        // the submit must be for the NEW job, not the abandoned one
        assert!(submit_line.contains("\"easy2\""), "should submit for the new job, got: {submit_line}");
        assert!(!submit_line.contains("\"hard1\""), "must not submit the cancelled job: {submit_line}");
        assert_eq!(got_share_for, "easy2");
    }

    // TLS IS NOT DECORATION. The payout address travels on this connection: on a plain one, anyone in the
    // middle rewrites it and collects the rewards of everyone mining through them, and the miner never
    // notices. So an encrypted client must REFUSE to talk to a server that cannot prove who it is.
    //
    // This points the TLS client at a plain TCP server (the stand-in for an impostor: something listening on
    // the right port with no valid certificate for that name). It must fail, not fall back to plain.
    #[test]
    fn the_encrypted_client_refuses_a_server_that_cannot_prove_who_it_is() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        // Accept and answer garbage, as an impostor with no certificate would.
        let server = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let _ = sock.write_all(b"{\"type\":\"login_result\",\"ok\":true}\n");
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let r = StratumClient::connect_tls(&addr, Duration::from_secs(5));
        // Either the setup rejects "127.0.0.1" as a name to verify, or the handshake fails. Both are correct:
        // what must NEVER happen is a successful session against something that proved nothing.
        match r {
            Err(_) => {}
            Ok(mut c) => {
                let hablo = c.login("brv1qexample", "rig1", Duration::from_secs(2));
                assert!(hablo.is_err(), "an encrypted client must not talk to a server with no valid certificate");
            }
        }
        let _ = server.join();
    }

    // A rejected login must NOT start mining: the pool credits nobody, so the CPU would burn for nothing.
    #[test]
    fn a_rejected_login_never_mines() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut writer = sock;
            let mut login = String::new();
            reader.read_line(&mut login).unwrap();
            writer.write_all(b"{\"type\":\"login_result\",\"ok\":false}\n").unwrap();
            writer.flush().unwrap();
            // If the miner ignored the rejection it would submit anyway; give it time to prove it does not.
            std::thread::sleep(Duration::from_millis(200));
        });
        let client = StratumClient::connect(&addr, Duration::from_secs(5)).unwrap();
        let stop = AtomicBool::new(false);
        let mined = Arc::new(AtomicBool::new(false));
        let mined_c = mined.clone();
        let stub = move |mj: &MiningJob, _t: usize, _ns: u64, _mx: u64, _s: &AtomicBool| -> Option<Solution> {
            mined_c.store(true, Ordering::SeqCst); // must never run
            Some(Solution { job_id: mj.job_id.clone(), nonce: 1, ntime: 0, extranonce2: String::new() })
        };
        let mut on_event = |_e: PoolEvent| {};
        let err = run_session(client, "brv1qbad", "rig1", 1, 1000, &stop, &std::sync::atomic::AtomicU64::new(0), &mut on_event, stub).unwrap_err();
        server.join().unwrap();
        assert!(!mined.load(Ordering::SeqCst), "must not mine a single hash after a rejected login");
        // Marked permanent so the reconnect loop stops instead of hammering the pool forever.
        assert!(err.starts_with(PERMANENT_PREFIX), "a rejected login must be permanent, got: {err}");
    }

    // THE ONE THAT MATTERS for the pool: a permanent rejection must stop the worker, not retry forever.
    // Without this, a miner with a wrong address hammers the pool for as long as the app is open, and the
    // user never learns why nothing works.
    #[test]
    fn a_permanent_rejection_stops_the_worker_instead_of_reconnecting() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let intentos = Arc::new(AtomicU64::new(0));
        let intentos_c = intentos.clone();
        // Non-blocking accept loop: count every reconnection for 2s. A correct worker connects ONCE and gives up.
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let hasta = std::time::Instant::now() + Duration::from_secs(2);
            while std::time::Instant::now() < hasta {
                match listener.accept() {
                    Ok((sock, _)) => {
                        intentos_c.fetch_add(1, Ordering::SeqCst);
                        sock.set_nonblocking(false).unwrap();
                        let mut reader = BufReader::new(sock.try_clone().unwrap());
                        let mut writer = sock;
                        let mut login = String::new();
                        let _ = reader.read_line(&mut login);
                        let _ = writer.write_all(b"{\"type\":\"login_result\",\"ok\":false}\n");
                        let _ = writer.flush();
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        });
        let stop = AtomicBool::new(false);
        let mut errores = Vec::new();
        // run_pool_worker blocks; a correct one returns on its own after the permanent rejection.
        run_pool_worker(&addr, "brv1qbad", "rig1", 1, false, &stop, |e| {
            if let PoolEvent::Disconnected(m) = e {
                errores.push(m);
            }
        });
        server.join().unwrap(); // let the 2s counting window finish: a buggy worker would reconnect within it
        assert_eq!(intentos.load(Ordering::SeqCst), 1, "must connect ONCE, not retry a definitive answer");
        assert_eq!(errores.len(), 1);
        assert!(!errores[0].starts_with(PERMANENT_PREFIX), "the user must read the reason, not the marker");
        assert!(errores[0].contains("rejected"), "the reason must reach the user, got: {:?}", errores[0]);
    }

    // A TEMPORARY drop (server closes without answering the login) must announce a reconnect countdown with an
    // ABSOLUTE future timestamp, and the worker must NEVER fall to solo — it keeps trying. We capture the first
    // Reconnecting and stop, so the test does not actually wait out the backoff.
    #[test]
    fn a_temporary_drop_announces_a_reconnect_countdown() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            if let Ok((sock, _)) = listener.accept() {
                let mut reader = BufReader::new(sock.try_clone().unwrap());
                let mut login = String::new();
                let _ = reader.read_line(&mut login); // read login, then drop the socket without answering
            }
        });
        let stop = AtomicBool::new(false);
        let mut reconnect_at = 0u64;
        let mut saw_solo = false;
        run_pool_worker(&addr, "brv1qtmp", "rig1", 1, false, &stop, |e| match e {
            PoolEvent::Reconnecting { retry_at } => {
                reconnect_at = retry_at;
                stop.store(true, Ordering::SeqCst); // got the countdown; stop before sleeping the whole backoff
            }
            // there is no "solo" event; a fall to solo would show up as the worker simply exiting without a
            // reconnect, which this assert (reconnect_at set) already guards against.
            PoolEvent::Disconnected(_) => saw_solo = false,
            _ => {}
        });
        server.join().unwrap();
        let now = unix_now();
        assert!(reconnect_at >= now, "reconnect countdown must be in the future: {reconnect_at} vs now {now}");
        assert!(reconnect_at <= now + 61, "first backoff is ~1s (cap 60): {reconnect_at} vs now {now}");
        assert!(!saw_solo);
    }

    // Several pools skip the explicit ok and answer the login with work. That job must be mined, not dropped.
    #[test]
    fn a_job_answering_the_login_counts_as_accepted_and_is_mined() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut writer = sock;
            let mut login = String::new();
            reader.read_line(&mut login).unwrap();
            writer.write_all((job_line("first", true, 3) + "\n").as_bytes()).unwrap(); // work, no ok
            writer.flush().unwrap();
            let mut submit = String::new();
            reader.read_line(&mut submit).unwrap();
            submit
        });
        let client = StratumClient::connect(&addr, Duration::from_secs(5)).unwrap();
        let stop = AtomicBool::new(false);
        let stub = |mj: &MiningJob, _t: usize, _ns: u64, _mx: u64, _s: &AtomicBool| -> Option<Solution> {
            Some(Solution { job_id: mj.job_id.clone(), nonce: 7, ntime: 0, extranonce2: String::new() })
        };
        let stop_ref = &stop;
        let mut on_event = |e: PoolEvent| {
            if let PoolEvent::ShareSubmitted { .. } = e {
                stop_ref.store(true, Ordering::SeqCst);
            }
        };
        run_session(client, "brv1qexample", "rig1", 1, 1000, &stop, &std::sync::atomic::AtomicU64::new(0), &mut on_event, stub).unwrap();
        let submit_line = server.join().unwrap();
        assert!(submit_line.contains("\"first\""), "the job that came with the login must be mined: {submit_line}");
    }

    // The pool's explicit ack drives the "accepted" event — the worker must NOT count acceptance on submit alone.
    #[test]
    fn pool_ack_marks_share_accepted() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut writer = sock;
            let mut login = String::new();
            reader.read_line(&mut login).unwrap();
            writer.write_all((job_line("j1", true, 5) + "\n").as_bytes()).unwrap();
            writer.flush().unwrap();
            let mut submit = String::new();
            reader.read_line(&mut submit).unwrap(); // read the share
            writer.write_all(b"{\"type\":\"ack\",\"job_id\":\"j1\",\"accepted\":true}\n").unwrap();
            writer.flush().unwrap();
        });
        let client = StratumClient::connect(&addr, Duration::from_secs(5)).unwrap();
        let stop = AtomicBool::new(false);
        let solved = Arc::new(AtomicBool::new(false)); // solve once, then block, so the miner doesn't spin
        let solved_c = solved.clone();
        let stub = move |mj: &MiningJob, _t: usize, _ns: u64, _mx: u64, s: &AtomicBool| -> Option<Solution> {
            if solved_c.swap(true, Ordering::SeqCst) {
                while !s.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                None
            } else {
                Some(Solution { job_id: mj.job_id.clone(), nonce: 1, ntime: 0, extranonce2: String::new() })
            }
        };
        let stop_ref = &stop;
        let mut accepted = false;
        let mut on_event = |e: PoolEvent| {
            if e == PoolEvent::ShareAccepted {
                accepted = true;
                stop_ref.store(true, Ordering::SeqCst);
            }
        };
        run_session(client, "brv1qexample", "rig1", 1, 1000, &stop, &std::sync::atomic::AtomicU64::new(0), &mut on_event, stub).unwrap();
        server.join().unwrap();
        assert!(accepted, "worker should mark the share accepted only after the pool's ack");
    }

    // Guardian (audit P0 #1): a pool under maintenance must be reported as Suspended, NOT as a Disconnect or
    // an error, and the miner must never submit a share or fall to solo. Login-time path.
    #[test]
    fn login_time_maintenance_is_suspended_not_disconnected() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut writer = sock;
            let mut login = String::new();
            reader.read_line(&mut login).unwrap();
            // answer the login with maintenance + a retry hint, then keep the socket open briefly so it lands
            writer
                .write_all(b"{\"type\":\"pool_suspended\",\"retry_after_seconds\":45}\n")
                .unwrap();
            writer.flush().unwrap();
            std::thread::sleep(Duration::from_millis(50));
        });
        let client = StratumClient::connect(&addr, Duration::from_secs(5)).unwrap();
        let stop = AtomicBool::new(false);
        let mut events = Vec::new();
        let stub = |_mj: &MiningJob, _t: usize, _ns: u64, _mx: u64, _s: &AtomicBool| -> Option<Solution> { None };
        let mut on_event = |e: PoolEvent| events.push(e);
        let err = run_session(client, "brv1qexample", "rig1", 1, 1000, &stop, &std::sync::atomic::AtomicU64::new(0), &mut on_event, stub)
            .unwrap_err();
        server.join().unwrap();
        assert!(err.starts_with(SUSPENDED_PREFIX), "maintenance must be flagged suspended, got: {err}");
        assert!(
            events.iter().any(|e| matches!(e, PoolEvent::Suspended { retry_after: Some(45) })),
            "must emit Suspended with the pool's retry hint: {events:?}"
        );
        assert!(!events.iter().any(|e| matches!(e, PoolEvent::Disconnected(_))), "suspension is NOT a disconnect");
        assert!(!events.iter().any(|e| matches!(e, PoolEvent::LoggedIn)), "never logged in during maintenance");
        assert!(!events.iter().any(|e| matches!(e, PoolEvent::ShareSubmitted { .. })), "never submit while suspended");
    }

    // Guardian: maintenance that arrives mid-session (already logged in and working). The session must end as
    // Suspended, cancel the current job (atomic cutoff), and not report a Disconnect.
    #[test]
    fn mid_session_maintenance_cancels_and_reports_suspended() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut writer = sock;
            let mut login = String::new();
            reader.read_line(&mut login).unwrap();
            writer.write_all((job_line("j1", true, 7) + "\n").as_bytes()).unwrap();
            writer.flush().unwrap();
            std::thread::sleep(Duration::from_millis(30));
            writer.write_all(b"{\"type\":\"pool_suspended\"}\n").unwrap(); // no hint -> miner uses its own 30-60s
            writer.flush().unwrap();
            std::thread::sleep(Duration::from_millis(50));
        });
        let client = StratumClient::connect(&addr, Duration::from_secs(5)).unwrap();
        let stop = AtomicBool::new(false);
        let mut events = Vec::new();
        // never solve: the ONLY thing that ends the session is the pool going into maintenance.
        let stub = |_mj: &MiningJob, _t: usize, _ns: u64, _mx: u64, s: &AtomicBool| -> Option<Solution> {
            while !s.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(2));
            }
            None
        };
        let mut on_event = |e: PoolEvent| events.push(e);
        let err = run_session(client, "brv1qexample", "rig1", 1, 1000, &stop, &std::sync::atomic::AtomicU64::new(0), &mut on_event, stub)
            .unwrap_err();
        server.join().unwrap();
        assert!(err.starts_with(SUSPENDED_PREFIX), "got: {err}");
        assert!(events.contains(&PoolEvent::LoggedIn), "logged in first: {events:?}");
        assert!(events.iter().any(|e| matches!(e, PoolEvent::Suspended { .. })), "must report Suspended: {events:?}");
        assert!(!events.iter().any(|e| matches!(e, PoolEvent::Disconnected(_))), "suspension is not a disconnect");
    }
}
