//! Talk to the REAL pool over TLS, from outside, the way the miner does.
//!
//! Why a separate binary and not a test: this needs the internet and the live pool, so it must never run in
//! CI (it would fail on any network hiccup and teach everyone to ignore red). It is the check you run by hand
//! after touching the pool's TLS, and before the launch.
//!
//! What it proves, in order:
//!   1. The encrypted connection is established (a plain one would fail here).
//!   2. The pool ANSWERS the login (before, it answered a good login with silence and the miner looped).
//!   3. The answer is coherent: either work, or an explicit rejection with a reason.
//!
//! Run:  cargo run --bin pool-tls-check -- [host:port] [address]
//!       default host: pool.brisvia.com:3333

use brisvia_randomx::stratum::{Incoming, LoginError, Poll, StratumClient};
use std::time::Duration;

fn main() {
    let pool = std::env::args().nth(1).unwrap_or_else(|| "pool.brisvia.com:3333".to_string());
    // The testnet pool only accepts "tbrv1..." addresses; the mainnet one only "brv1...". A rejection with a
    // clear reason is a PASS here: it proves the encrypted round trip works end to end.
    let addr = std::env::args().nth(2).unwrap_or_else(|| "tbrv1qcheckcheckcheckcheckcheckcheckcheck".to_string());

    println!("1) connecting ENCRYPTED to {pool} ...");
    let mut c = match StratumClient::connect_tls(&pool, Duration::from_secs(20)) {
        Ok(c) => {
            println!("   OK: encrypted connection established and certificate verified");
            c
        }
        Err(e) => {
            eprintln!("   FAIL: could not establish the encrypted connection -> {e}");
            eprintln!("   (if the pool still speaks in the clear, this fails: that is the correct signal)");
            std::process::exit(1);
        }
    };

    println!("2) login (ONLY the public payout address travels) ...");
    match c.login(&addr, "tls-check", Duration::from_secs(20)) {
        Ok(Some(job)) => {
            println!("   OK: the pool accepted and sent work");
            println!("      job_id={} height={} header={} chars", job.job_id, job.height, job.header_template.len());
            match job.to_mining_job() {
                Ok(_) => println!("   OK: the work is valid and mineable"),
                Err(e) => {
                    eprintln!("   FAIL: the pool sent work the miner cannot use -> {e}");
                    std::process::exit(1);
                }
            }
        }
        Ok(None) => {
            println!("   OK: the pool confirmed the login explicitly");
            // A confirmed login is not enough: the work must arrive afterwards, or the miner stays
            // connected without mining. We wait for the first job just like the real session does.
            println!("3) waiting for the first job ...");
            let hasta = std::time::Instant::now() + Duration::from_secs(30);
            let mut llego = false;
            while std::time::Instant::now() < hasta && !llego {
                match c.poll_message(Duration::from_millis(500)) {
                    Ok(Poll::Message(Incoming::Job(j))) => {
                        println!("   OK: work arrived -> job_id={} height={}", j.job_id, j.height);
                        match j.to_mining_job() {
                            Ok(_) => println!("   OK: the work is valid and mineable"),
                            Err(e) => {
                                eprintln!("   FAIL: work the miner cannot use -> {e}");
                                std::process::exit(1);
                            }
                        }
                        llego = true;
                    }
                    Ok(Poll::Closed) => {
                        eprintln!("   FAIL: the pool closed the connection without sending work");
                        std::process::exit(1);
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        eprintln!("   FAIL: error reading from the pool -> {e}");
                        std::process::exit(1);
                    }
                }
            }
            if !llego {
                eprintln!("   FAIL: the pool confirmed the login but sent no work in 30s");
                std::process::exit(1);
            }
        }
        Err(LoginError::Permanent(m)) => {
            println!("   OK: the pool answered and explicitly REJECTED -> {m}");
            println!("      (expected if the address does not match this pool's network)");
        }
        Err(LoginError::Temporary(m)) => {
            eprintln!("   FAIL: the pool did not give a clear answer -> {m}");
            std::process::exit(1);
        }
    }
    c.shutdown();
    println!("\nOK: the encrypted miner -> pool path works end to end.");
}
