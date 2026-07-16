//! E2E test binary: run the real pool worker against a live stratum pool, print every event.
//! Usage: pool_e2e <payout_address> [host:port]  (defaults to the official pool).
//! Stops after 3 accepted shares or a timeout, whichever comes first.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use brisvia_randomx::pool_worker::{run_pool_worker, PoolEvent};

fn main() {
    let addr = std::env::args().nth(1).expect("usage: pool_e2e <address> [host:port]");
    let pool = std::env::args().nth(2).unwrap_or_else(|| "pool.brisvia.com:3333".to_string());
    let target_shares: usize = 3;
    let timeout = Duration::from_secs(120);

    let should_stop = Arc::new(AtomicBool::new(false));
    let accepted = Arc::new(AtomicUsize::new(0));

    // Watchdog: stop after the timeout no matter what.
    {
        let ss = should_stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            ss.store(true, Ordering::SeqCst);
        });
    }

    println!("== E2E pool worker ==  pool={pool}  address={addr}");
    let acc = accepted.clone();
    let ss = should_stop.clone();
    let start = Instant::now();
    let on_event = move |ev: PoolEvent| {
        println!("[{:6.1}s] {:?}", start.elapsed().as_secs_f32(), ev);
        if ev == PoolEvent::ShareAccepted && acc.fetch_add(1, Ordering::SeqCst) + 1 >= target_shares {
            ss.store(true, Ordering::SeqCst);
        }
    };
    // The e2e talks to a local loopback pool, which has no certificate: it goes in the clear on purpose.
    run_pool_worker(&pool, &addr, "e2e", 2, false, &should_stop, on_event);
    println!("== END ==  accepted shares: {}", accepted.load(Ordering::SeqCst));
}
