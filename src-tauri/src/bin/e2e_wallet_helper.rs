// E2E migration helper — built ONLY with `--features e2e`, NEVER shipped in the public app (see the
// [[bin]] required-features guard in Cargo.toml and the e2e_helper module in lib.rs).
//
// It lets the macOS/Linux migration jobs create and later verify a THROWAWAY wallet using the app's REAL
// crypto, so the wallet_seed.enc is byte-compatible with what the app writes. Secrets (the throwaway
// mnemonic and password) are read from STDIN — never from argv (which shows up in the process list) or
// printed to logs. The ONLY thing printed is the wallet's public receiving address, which is safe to log.
//
//   create <datadir>   stdin: line 1 = mnemonic, line 2 = password  -> prints the first address
//   verify <datadir>   stdin: line 1 = password                     -> prints the first address
//
// A migration test creates a wallet (address A), runs the real 1.0.7 then 1.0.8 packages over the same data
// directory, then verifies (address A'): A == A' proves the surviving seed still unlocks and derives the same
// address after the update.
use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: e2e-wallet-helper <create|verify> <datadir>   (secrets via stdin)");
        std::process::exit(2);
    }
    let cmd = args[1].as_str();
    let datadir = std::path::PathBuf::from(&args[2]);

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).expect("read stdin");
    let mut lines = input.lines();

    let result = match cmd {
        "create" => {
            let mnemonic = lines.next().unwrap_or("").trim();
            let password = lines.next().unwrap_or("").trim();
            brisvia_miner_lib::e2e_helper::create(&datadir, mnemonic, password)
        }
        "verify" => {
            let password = lines.next().unwrap_or("").trim();
            brisvia_miner_lib::e2e_helper::verify(&datadir, password)
        }
        other => {
            eprintln!("unknown command: {other}");
            std::process::exit(2);
        }
    };

    match result {
        Ok(address) => println!("{address}"),
        Err(e) => {
            eprintln!("ERR: {e}");
            std::process::exit(1);
        }
    }
}
