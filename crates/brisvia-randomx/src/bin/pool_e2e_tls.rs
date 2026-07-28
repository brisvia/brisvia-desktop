//! Same as pool_e2e, but over TLS — the way the official pool is actually reached.
//!
//! WHY THIS EXISTS: pool_e2e connects in plaintext, so every client test done with it exercised a
//! path no real miner will ever use. The official pool terminates TLS at the edge. This binary calls
//! the SAME run_pool_worker with tls = true, so the encrypted path gets the same coverage.
//!
//! Usage: pool_e2e_tls <payout_address> [host:port]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use brisvia_randomx::pool_worker::{run_pool_worker, PoolEvent};

fn main() {
    let addr = std::env::args().nth(1).expect("usage: pool_e2e_tls <address> [host:port]");
    let pool = std::env::args().nth(2).unwrap_or_else(|| "pool.brisvia.com:3333".to_string());
    let target_shares: usize = 3;
    let timeout = Duration::from_secs(180);

    let should_stop = Arc::new(AtomicBool::new(false));
    let accepted = Arc::new(AtomicUsize::new(0));
    {
        let ss = should_stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            ss.store(true, Ordering::SeqCst);
        });
    }

    println!("== E2E pool worker over TLS ==  pool={pool}  address={addr}");
    let acc = accepted.clone();
    let ss = should_stop.clone();
    let started = Instant::now();
    let on_event = move |e: PoolEvent| {
        println!("[{:6.1}s] {e:?}", started.elapsed().as_secs_f64());
        if matches!(e, PoolEvent::ShareAccepted)
            && acc.fetch_add(1, Ordering::SeqCst) + 1 >= target_shares
        {
            ss.store(true, Ordering::SeqCst);
        }
    };

    // tls = true: this is the whole point of this binary.
    run_pool_worker(&pool, &addr, "e2e-tls", 2, true, &should_stop, on_event);
    println!("== END ==  accepted shares: {}", accepted.load(Ordering::SeqCst));
}
