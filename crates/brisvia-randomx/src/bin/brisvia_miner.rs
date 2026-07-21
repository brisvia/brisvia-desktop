//! Brisvia terminal miner — mine to the official pool from a machine with no desktop.
//!
//! WHY THIS EXISTS: the published Linux builds are desktop apps (.AppImage and .deb). Anyone running
//! a headless server — a large share of CPU miners — had no way to mine at all. This drives the SAME
//! engine the desktop app uses (`run_pool_worker`); only the way you start it is different.
//!
//! It has LESS reach than the desktop app, on purpose: no wallet, no recovery phrase, no bundled
//! node. It takes a payout address, and a payout address can only receive.
//!
//! DELIBERATE CHOICES, each one there for a reason:
//!   - TLS is always on and the certificate is always checked. There is no flag to weaken either:
//!     the address you get paid to travels on this link, and on a server nobody is watching the
//!     screen to notice a warning.
//!   - The payout address is fully decoded and checksum-verified before a single hash is computed.
//!     A typo that still "looks like" an address would otherwise send weeks of mining to someone
//!     else, irreversibly.
//!   - Individual shares are NOT logged by default. At 20 shares a second that is 1.7 million lines
//!     a day on a machine that may run for weeks.
//!   - If the worker stops on its own, the process exits non-zero, so a service manager restarts it
//!     instead of assuming the miner finished its job.
//!
//! Usage:
//!   brisvia-miner --address <BRV_ADDRESS> [--threads N] [--pool host:port] [--worker NAME]
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use brisvia_randomx::pool_worker::{run_pool_worker, PoolEvent};

const DEFAULT_POOL: &str = "pool.brisvia.com:3333";
const HRP: &str = "brv";
/// How often the summary line is printed. Anything more frequent fills the log of someone who
/// leaves this running for weeks.
const SUMMARY_EVERY: Duration = Duration::from_secs(60);
/// Server-provided text is truncated to this before printing. A hostile or broken pool could
/// otherwise send terminal escape sequences or megabytes of text and forge what the log looks like.
const MAX_SERVER_TEXT: usize = 120;

fn print_help(code: i32) -> ! {
    // La ayuda pedida a proposito va por la salida normal y con codigo 0. Un error de argumentos va
    // por la salida de errores y con codigo 2: asi un guion puede distinguirlos.
    let texto = format!(
        "Brisvia terminal miner

USAGE:
  brisvia-miner --address <YOUR_BRV_ADDRESS> [OPTIONS]

OPTIONS:
  -a, --address <ADDR>    Where your rewards are paid. Required. A Brisvia address ('brv1...').
  -t, --threads <N>       CPU threads to mine with. Default: cores minus one.
  -p, --pool <HOST:PORT>  Pool to connect to. Default: {DEFAULT_POOL}
  -w, --worker <NAME>     A name for this machine, to tell your miners apart. Default: cli
      --verbose-shares    Print every single share. Off by default: it floods the log.
  -h, --help              Show this.

EXAMPLE:
  brisvia-miner --address brv1qexample... --threads 4

The connection is always encrypted and the certificate is always checked. There is no option to
disable either: your payout address travels on this link.

The address format and checksum are verified before mining starts, which catches typos. It cannot
tell whether the address is yours: check that yourself, rewards cannot be recovered.

This miner never switches to solo mining by itself. If the pool goes down it says so, waits, and
reconnects.

Stop with Ctrl+C or `systemctl stop`. Shares already accepted stay credited.");
    if code == 0 { println!("{texto}"); } else { eprintln!("{texto}"); }
    std::process::exit(code)
}

fn argument_error(msg: &str) -> ! {
    eprintln!("{msg}
");
    print_help(2)
}

/// Decodes a Brisvia address completely: bech32/bech32m form, checksum, our prefix, witness version
/// and program length.
///
/// WHY NOT JUST CHECK THE PREFIX: `starts_with("brv1")` accepts a typo, a truncated address, or an
/// address copied from somewhere else. The pool would reject it — but only after the miner has been
/// burning CPU. Worse, an address that is well-formed but *wrong* is accepted by everyone and the
/// rewards go to a stranger, with no way back. The checksum exists precisely to catch that, so it is
/// checked here, before the first hash.
fn check_address(addr: &str) -> Result<(), String> {
    use bitcoin::bech32::primitives::decode::SegwitHrpstring;

    // SegwitHrpstring is the decoder meant for segwit addresses: it checks the checksum, reads the
    // witness version, and returns the program with the version already removed.
    //
    // THE FIRST VERSION OF THIS FUNCTION USED THE GENERIC DECODER and treated the first byte of the
    // payload as the witness version. That is not how a segwit address is encoded, and it REJECTED
    // real Brisvia addresses: a user with a perfectly good address would have been told it was
    // invalid and could not have mined at all. It only showed up by checking this against what the
    // pool itself accepts, address by address.
    if addr.trim() != addr {
        return Err("the address has leading or trailing spaces".into());
    }
    let parsed = SegwitHrpstring::new(addr).map_err(|_| {
        "this is not a valid address — check it character by character (one wrong letter is enough)"
            .to_string()
    })?;

    if parsed.hrp().to_lowercase() != HRP {
        return Err(format!(
            "this address is not a Brisvia address (it starts with '{}', ours start with '{HRP}')",
            parsed.hrp().to_lowercase()
        ));
    }

    // Same rule as the pool: witness v0 with a 20 or 32 byte program, or a later version with a
    // program between 2 and 40 bytes. The CLI must accept exactly what the pool accepts — no more,
    // so nobody mines to something that will be refused; no less, so nobody is locked out.
    let version = parsed.witness_version().to_u8();
    let program: Vec<u8> = parsed.byte_iter().collect();
    if version == 0 && program.len() != 20 && program.len() != 32 {
        return Err("this address has an invalid length".into());
    }
    if program.len() < 2 || program.len() > 40 {
        return Err("this address has an invalid length".into());
    }
    Ok(())
}


/// Strips control characters and truncates text that came from the server before it is printed.
fn safe(text: &str) -> String {
    let mut s: String = text
        .chars()
        // Solo ASCII imprimible: los caracteres de control no alcanzan, porque hay caracteres
        // Unicode que cambian el sentido de lectura y pueden usarse para disfrazar un registro.
        .map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { '?' })
        .take(MAX_SERVER_TEXT)
        .collect();
    if text.len() > MAX_SERVER_TEXT {
        s.push('…');
    }
    s
}

struct Args {
    address: String,
    pool: String,
    worker: String,
    threads: usize,
    verbose_shares: bool,
}

fn parse_args() -> Args {
    let mut address: Option<String> = None;
    let mut pool = DEFAULT_POOL.to_string();
    let mut worker = "cli".to_string();
    let mut threads: Option<usize> = None;
    let mut verbose_shares = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let mut value = |i: usize| -> String {
            match args.get(i + 1) {
                Some(v) if !v.starts_with('-') => v.clone(),
                _ => {
                    eprintln!("Missing value after {}\n", args[i]);
                    print_help(2)
                }
            }
        };
        match args[i].as_str() {
            "-a" | "--address" => {
                address = Some(value(i));
                i += 1;
            }
            "-p" | "--pool" => {
                pool = value(i);
                i += 1;
            }
            "-w" | "--worker" => {
                worker = value(i);
                i += 1;
            }
            "-t" | "--threads" => {
                let v = value(i);
                threads = Some(v.parse().unwrap_or_else(|_| {
                    eprintln!("--threads must be a whole number, got: {v}\n");
                    print_help(2)
                }));
                i += 1;
            }
            "--verbose-shares" => verbose_shares = true,
            "-h" | "--help" => print_help(0),
            other => {
                eprintln!("Unknown option: {other}\n");
                print_help(2)
            }
        }
        i += 1;
    }

    let address = address.unwrap_or_else(|| {
        eprintln!("Missing --address. Run with --help to see how to use this.\n");
        print_help(2)
    });
    if let Err(why) = check_address(&address) {
        eprintln!("The payout address is not valid: {why}");
        eprintln!("\nNothing was mined. Fix the address and try again — rewards sent to a wrong \
                   address cannot be recovered.");
        std::process::exit(2);
    }

    // A miner that makes the machine unusable gets turned off, which helps nobody. And an absurd
    // thread count would just exhaust memory: each RandomX thread holds its own VM.
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let threads = match threads {
        None => cores.saturating_sub(1).max(1),
        Some(0) => {
            eprintln!("--threads must be at least 1\n");
            print_help(2)
        }
        Some(n) if n > cores => {
            eprintln!(
                "--threads {n} is more than this machine's {cores} CPU threads. Using more than \
                 you have makes mining slower, not faster.\n"
            );
            print_help(2)
        }
        Some(n) => n,
    };

    Args { address, pool, worker, threads, verbose_shares }
}

fn main() {
    let a = parse_args();
    let should_stop = Arc::new(AtomicBool::new(false));
    install_signal_handlers(should_stop.clone());

    println!("Brisvia miner");
    println!("  pool     : {}  (encrypted, certificate checked)", a.pool);
    println!("  address  : {}  (format and checksum verified)", a.address);
    println!("             Check this is YOUR address: the miner cannot know whose it is.");
    println!("  threads  : {}", a.threads);
    println!("  Stop with Ctrl+C.\n");

    let accepted = Arc::new(AtomicUsize::new(0));
    let rejected = Arc::new(AtomicUsize::new(0));
    let stale = Arc::new(AtomicUsize::new(0));
    let blocks = Arc::new(AtomicUsize::new(0));
    // Rejections are counted by cause and reported in the summary. The first of each kind is printed
    // right away — that is the one that tells you something is wrong — and the rest are counted.
    let reasons: Arc<Mutex<BTreeMap<String, usize>>> = Arc::new(Mutex::new(BTreeMap::new()));
    let started = Instant::now();
    let last_summary = Arc::new(Mutex::new(Instant::now()));

    let (acc, rej, stl, blk, rsn, verbose) =
        (accepted.clone(), rejected.clone(), stale.clone(), blocks.clone(),
         reasons.clone(), a.verbose_shares);

    let on_event = move |e: PoolEvent| match e {
        PoolEvent::Connected => println!("[{}] connected", clock(started)),
        PoolEvent::LoggedIn => println!("[{}] logged in, waiting for work", clock(started)),
        PoolEvent::NewJob { height, .. } => {
            println!("[{}] new work (block height {height})", clock(started))
        }
        PoolEvent::ShareAccepted => {
            let n = acc.fetch_add(1, Ordering::Relaxed) + 1;
            if verbose {
                println!("[{}] share accepted  (total {n})", clock(started));
            }
        }
        PoolEvent::ShareRejected { reason } => {
            rej.fetch_add(1, Ordering::Relaxed);
            let motivo = safe(&reason);
            let mut m = rsn.lock().unwrap();
            // Tope de causas distintas. `--pool` permite apuntar a cualquier servidor, asi que no se
            // puede suponer que los motivos vengan de una lista corta: sin tope, una pool rota u
            // hostil haria crecer este mapa sin limite.
            let clave = if m.len() >= 24 && !m.contains_key(&motivo) {
                "other".to_string()
            } else {
                motivo.clone()
            };
            let motivo = clave.clone();
            let n = m.entry(clave).or_insert(0);
            *n += 1;
            // Solo el primero de cada tipo se muestra al instante: es el que avisa que algo pasa.
            if *n == 1 || verbose {
                println!("[{}] share rejected: {motivo}", clock(started));
            }
        }
        PoolEvent::ShareStale => {
            stl.fetch_add(1, Ordering::Relaxed);
        }
        PoolEvent::BlockFound => {
            blk.fetch_add(1, Ordering::Relaxed);
            println!(
                "[{}] *** CANDIDATE BLOCK FOUND *** the pool is submitting it",
                clock(started)
            );
        }
        PoolEvent::Suspended { retry_after } => println!(
            "[{}] the pool is under maintenance. Retrying in {} s. You are NOT switched to solo \
             mining — nothing changes without you asking.",
            clock(started),
            retry_after.unwrap_or(60)
        ),
        PoolEvent::Reconnecting { .. } => {
            println!("[{}] lost the connection, reconnecting", clock(started))
        }
        PoolEvent::Disconnected(why) => {
            let w = safe(&why);
            if w.is_empty() {
                println!("[{}] disconnected", clock(started));
            } else {
                println!("[{}] disconnected: {w}", clock(started));
            }
        }
        PoolEvent::Hashrate(h) => {
            let mut last = last_summary.lock().unwrap();
            if last.elapsed() >= SUMMARY_EVERY {
                *last = Instant::now();
                let r = rej.load(Ordering::Relaxed);
                let mut line = format!(
                    "[{}] {h:.0} hashes/s   accepted {}   rejected {}   late {}",
                    clock(started),
                    acc.load(Ordering::Relaxed),
                    r,
                    stl.load(Ordering::Relaxed)
                );
                if blk.load(Ordering::Relaxed) > 0 {
                    line.push_str(&format!("   block candidates {}", blk.load(Ordering::Relaxed)));
                }
                if r > 0 {
                    let m = rsn.lock().unwrap();
                    let detalle: Vec<String> =
                        m.iter().map(|(k, v)| format!("{k} x{v}")).collect();
                    line.push_str(&format!("   [{}]", detalle.join(", ")));
                }
                println!("{line}");
            }
        }
        _ => {}
    };

    // tls = true, always. There is deliberately no way to ask for anything else.
    run_pool_worker(&a.pool, &a.address, &a.worker, a.threads, true, &should_stop, on_event);

    let resumen = format!(
        "after {}. Accepted {}, rejected {}, late {}, block candidates {}.",
        clock(started),
        accepted.load(Ordering::Relaxed),
        rejected.load(Ordering::Relaxed),
        stale.load(Ordering::Relaxed),
        blocks.load(Ordering::Relaxed)
    );

    // If the worker came back on its own, this was NOT a clean stop. Exiting 0 here would make a
    // service manager believe the miner finished its job and leave the machine idle.
    if !should_stop.load(Ordering::SeqCst) {
        eprintln!("\nThe mining worker stopped unexpectedly {resumen}");
        std::process::exit(1);
    }
    println!("\nStopped {resumen}");
}

fn clock(since: Instant) -> String {
    let s = since.elapsed().as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// Ctrl+C and `systemctl stop` (SIGTERM) stop the miner cleanly instead of killing it mid-share.
///
/// A dedicated thread waits on the signals instead of doing work inside a signal handler: almost
/// nothing is safe to call from a handler, and the previous version reached into a shared cell from
/// there.
#[cfg(unix)]
fn install_signal_handlers(stop: Arc<AtomicBool>) {
    use std::os::unix::net::UnixStream;
    // A self-pipe: the handler only writes one byte, which is safe. The thread does the real work.
    let (mut r, w) = match UnixStream::pair() {
        Ok(p) => p,
        Err(_) => return,
    };
    let fd = {
        use std::os::unix::io::IntoRawFd;
        w.into_raw_fd()
    };
    unsafe {
        SIGNAL_FD = fd;
        set_handler(2); // SIGINT  — Ctrl+C
        set_handler(15); // SIGTERM — systemctl stop
    }
    std::thread::spawn(move || {
        use std::io::Read;
        let mut b = [0u8; 1];
        if r.read_exact(&mut b).is_ok() {
            stop.store(true, Ordering::SeqCst);
        }
    });
}

#[cfg(unix)]
static mut SIGNAL_FD: i32 = -1;

#[cfg(unix)]
unsafe fn set_handler(sig: i32) {
    extern "C" {
        fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
    }
    extern "C" fn on_signal(_: i32) {
        // The only thing done from inside the handler is a write() of one byte, which is one of the
        // few calls guaranteed to be safe here.
        extern "C" {
            fn write(fd: i32, buf: *const u8, n: usize) -> isize;
        }
        unsafe {
            let b: u8 = 1;
            let _ = write(SIGNAL_FD, &b as *const u8, 1);
        }
    }
    signal(sig, on_signal);
}

#[cfg(not(unix))]
fn install_signal_handlers(_stop: Arc<AtomicBool>) {
    // On Windows the default Ctrl+C behaviour ends the process, and there is no service case here.
}
