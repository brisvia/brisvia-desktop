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

use brisvia_randomx::stratum::{LoginError, StratumClient};
use std::time::Duration;

fn main() {
    let pool = std::env::args().nth(1).unwrap_or_else(|| "pool.brisvia.com:3333".to_string());
    // The testnet pool only accepts "tbrv1..." addresses; the mainnet one only "brv1...". A rejection with a
    // clear reason is a PASS here: it proves the encrypted round trip works end to end.
    let addr = std::env::args().nth(2).unwrap_or_else(|| "tbrv1qcheckcheckcheckcheckcheckcheckcheck".to_string());

    println!("1) conectando CIFRADO a {pool} ...");
    let mut c = match StratumClient::connect_tls(&pool, Duration::from_secs(20)) {
        Ok(c) => {
            println!("   OK: conexion cifrada establecida y certificado verificado");
            c
        }
        Err(e) => {
            eprintln!("   FALLA: no se pudo establecer la conexion cifrada -> {e}");
            eprintln!("   (si la pool todavia habla en claro, esto falla: es la senal correcta)");
            std::process::exit(1);
        }
    };

    println!("2) login (viaja SOLO la direccion publica de pago) ...");
    match c.login(&addr, "tls-check", Duration::from_secs(20)) {
        Ok(Some(job)) => {
            println!("   OK: la pool acepto y mando trabajo");
            println!("      job_id={} altura={} header={} chars", job.job_id, job.height, job.header_template.len());
            match job.to_mining_job() {
                Ok(_) => println!("   OK: el trabajo es valido y minable"),
                Err(e) => {
                    eprintln!("   FALLA: la pool mando un trabajo que el minero no puede usar -> {e}");
                    std::process::exit(1);
                }
            }
        }
        Ok(None) => println!("   OK: la pool acepto el login (sin trabajo listo todavia: espera un job)"),
        Err(LoginError::Permanent(m)) => {
            println!("   OK: la pool respondio y RECHAZO explicitamente -> {m}");
            println!("      (esperado si la direccion no corresponde a la red de esta pool)");
        }
        Err(LoginError::Temporary(m)) => {
            eprintln!("   FALLA: la pool no dio una respuesta clara -> {m}");
            std::process::exit(1);
        }
    }
    c.shutdown();
    println!("\nOK: el camino cifrado minero -> pool funciona de punta a punta.");
}
