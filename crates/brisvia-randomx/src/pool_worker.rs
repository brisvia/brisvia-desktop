//! Pool worker loop: connect to a stratum pool, log in, receive jobs, mine them, submit shares, reconnect.
//! Ties together `stratum` (protocol), `pool_miner` (the RandomX search) and `worksource` (the job type).
//! Approach D: the pool sends a ready header; the worker only varies the nonce and submits {job_id, nonce}.
//!
//! Cancellation (per ChatGPT's SECURITY review, P0): a reader loop receives jobs and raises a `cancel` flag on
//! every new job (or disconnect); the miner checks that flag (mine_job polls it every 256 hashes), so it drops a
//! dead job within milliseconds instead of scanning up to 50M nonces. Only the current generation's share is
//! submitted; a solution for a superseded job is discarded. The audited solo path is untouched.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::stratum::{Incoming, Poll, StratumClient};
use crate::worksource::{MiningJob, Solution};

/// One event the worker reports (the Tauri backend turns these into UI updates, like the solo worker).
/// Per ChatGPT: found-locally, submitted, and accepted are DISTINCT — the UI counts a contribution only on
/// `ShareAccepted` (the pool's explicit confirmation), never on submit.
#[derive(Debug, Clone, PartialEq)]
pub enum PoolEvent {
    Connected,
    LoggedIn,
    NewJob { job_id: String, height: u64 },
    ShareSubmitted { job_id: String, nonce: u32 }, // found locally and sent — NOT yet confirmed
    ShareAccepted,                                 // pool confirmed the share
    ShareRejected { reason: String },              // pool rejected (low diff, duplicate, unknown job…)
    ShareStale,                                    // arrived late (not fraud)
    BlockFound,                                    // our share met the network target → a block
    Disconnected(String),
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
    on_event: &mut F,
    mine: M,
) -> Result<(), String>
where
    F: FnMut(PoolEvent),
    M: Fn(&MiningJob, usize, u64, u64, &AtomicBool) -> Option<Solution> + Send,
{
    on_event(PoolEvent::Connected);
    client.login(address, worker)?;
    on_event(PoolEvent::LoggedIn);

    let job: Arc<Mutex<Option<(MiningJob, u64)>>> = Arc::new(Mutex::new(None));
    let cur_gen = Arc::new(AtomicU64::new(0));
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
                // Barrido progresivo de nonces por job: `start` avanza tras cada share para NO re-enviar el
                // mismo nonce (evita "duplicate") y para producir varias shares por job (clave en PPLNS).
                // Resets to 0 when a new job arrives (the generation changes).
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
                                    start = sol.nonce as u64 + 1; // continue AFTER the nonce found
                                }
                                None => {
                                    // Range exhausted or cancelled. If the job is still valid, wait for the next one
                                    // (do not re-sweep from 0, which would resend nonces already tried).
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
            loop {
                if should_stop.load(Ordering::Relaxed) {
                    return Ok(());
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
                    // Contract (avoids the block+ack double count ChatGPT flagged): the pool sends EXACTLY ONE
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
                    // A rejected login is permanent — stop this session with an error.
                    Poll::Message(Incoming::LoginResult { ok: false }) => return Err("login rejected".into()),
                    // v1 does not support mid-job target changes; each Job carries its own share_target.
                    Poll::Message(Incoming::SetTarget(_)) => {}
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
pub fn run_pool_worker<F>(host_port: &str, address: &str, worker: &str, threads: usize, should_stop: &AtomicBool, mut on_event: F)
where
    F: FnMut(PoolEvent),
{
    // One SeedGuard for the whole worker: it reuses the RandomX cache across jobs (and reconnections) and
    // rate-limits rebuilds so a hostile pool cannot force a rebuild loop. Shared into the per-session mine closure.
    let guard = Arc::new(Mutex::new(crate::pool_miner::SeedGuard::new()));
    let mut backoff = 1u64;
    while !should_stop.load(Ordering::Relaxed) {
        match StratumClient::connect(host_port, Duration::from_secs(15)) {
            Ok(client) => {
                backoff = 1; // reset after a good connection
                let guard = guard.clone();
                let mine = move |mj: &MiningJob, threads: usize, nonce_start: u64, max_nonces: u64, cancel: &AtomicBool| -> Option<Solution> {
                    // Reuse the cache while the seed is unchanged; refuse a rebuild burst (returns None → skip).
                    let cache = match guard.lock().unwrap().cache_for(&mj.seed_key) {
                        Ok(c) => c,
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(500)); // don't spin while churn is rate-limited
                            return None;
                        }
                    };
                    crate::pool_miner::mine_with_cache(mj, &cache, threads, nonce_start, max_nonces, cancel)
                };
                let res = run_session(client, address, worker, threads, 50_000_000, should_stop, &mut on_event, mine);
                on_event(PoolEvent::Disconnected(res.err().unwrap_or_default()));
            }
            Err(e) => on_event(PoolEvent::Disconnected(e)),
        }
        if should_stop.load(Ordering::Relaxed) {
            break;
        }
        // interruptible-ish backoff (checks stop each second)
        for _ in 0..backoff {
            if should_stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        backoff = (backoff * 2).min(60);
    }
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
        let stub = |mj: &MiningJob, _t: usize, _mx: u64, _s: &AtomicBool| -> Option<Solution> {
            Some(Solution { job_id: mj.job_id.clone(), nonce: 0x2a, ntime: 0, extranonce2: String::new() })
        };
        let stop_ref = &stop;
        let mut on_event = |e: PoolEvent| {
            if let PoolEvent::ShareSubmitted { .. } = e { stop_ref.store(true, Ordering::SeqCst); }
            events.push(e);
        };
        run_session(client, "brv1qexample", "rig1", 1, 1000, &stop, &mut on_event, stub).unwrap();
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
        let stub = |mj: &MiningJob, _t: usize, _mx: u64, s: &AtomicBool| -> Option<Solution> {
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
        run_session(client, "brv1qexample", "rig1", 1, 1000, &stop, &mut on_event, stub).unwrap();
        let submit_line = server.join().unwrap();
        // the submit must be for the NEW job, not the abandoned one
        assert!(submit_line.contains("\"easy2\""), "should submit for the new job, got: {submit_line}");
        assert!(!submit_line.contains("\"hard1\""), "must not submit the cancelled job: {submit_line}");
        assert_eq!(got_share_for, "easy2");
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
        let stub = move |mj: &MiningJob, _t: usize, _mx: u64, s: &AtomicBool| -> Option<Solution> {
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
        run_session(client, "brv1qexample", "rig1", 1, 1000, &stop, &mut on_event, stub).unwrap();
        server.join().unwrap();
        assert!(accepted, "worker should mark the share accepted only after the pool's ack");
    }
}
