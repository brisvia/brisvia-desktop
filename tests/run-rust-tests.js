// Runner for the backend (Rust) tests. Runs the suite on BOTH network builds:
//   - testnet (default): the wpkh guard requires tprv keys.
//   - mainnet (--features mainnet): the wpkh guard requires xprv keys.
// This way, the test `wallet_key_tests::ext_key_prefix_matches_build_network` guards the key bug
// ("wpkh(): key '...' is not valid") on both networks, with the REAL descriptor generator.
//
// Finds cargo even when it is not on the PATH (typical on this machine: rustup in ~/.cargo/bin).
'use strict';

const { spawnSync } = require('child_process');
const path = require('path');
const os = require('os');
const fs = require('fs');

function findCargo() {
  // 1) cargo on the PATH.
  const probe = spawnSync('cargo', ['--version'], { encoding: 'utf8', shell: false });
  if (probe.status === 0) return 'cargo';
  // 2) Fallback: rustup installation in the user's home.
  const exe = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
  const candidate = path.join(os.homedir(), '.cargo', 'bin', exe);
  if (fs.existsSync(candidate)) return candidate;
  return null;
}

const cargo = findCargo();
if (!cargo) {
  console.error('cargo (Rust) not found. Install Rust with rustup to run the backend tests.');
  process.exit(1);
}

const manifest = path.resolve(__dirname, '..', 'src-tauri', 'Cargo.toml');
const runs = [
  { label: 'backend (test network / testnet)', args: ['test', '--manifest-path', manifest] },
  { label: 'backend (real network / mainnet)', args: ['test', '--manifest-path', manifest, '--features', 'mainnet'] },
];

let failed = false;
for (const run of runs) {
  console.log('\n=== Tests ' + run.label + ' ===');
  const r = spawnSync(cargo, run.args, { stdio: 'inherit', shell: false });
  if (r.status !== 0) failed = true;
}

process.exit(failed ? 1 : 0);
