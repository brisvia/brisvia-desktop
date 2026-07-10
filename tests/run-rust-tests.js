// Runner de los tests del backend (Rust). Corre la suite en las DOS builds de red:
//   - testnet (por defecto): el guard del wpkh exige llaves tprv.
//   - mainnet (--features mainnet): el guard del wpkh exige llaves xprv.
// This way the test `wallet_key_tests::ext_key_prefix_matches_build_network` guards the key bug
// ("wpkh(): key '...' is not valid") en las dos redes, con el generador de descriptores REAL.
//
// Locates cargo even if it is not on the PATH (typical on this machine: rustup in ~/.cargo/bin).
'use strict';

const { spawnSync } = require('child_process');
const path = require('path');
const os = require('os');
const fs = require('fs');

function findCargo() {
  // 1) cargo en el PATH.
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
  { label: 'backend (red de prueba / testnet)', args: ['test', '--manifest-path', manifest] },
  { label: 'backend (red real / mainnet)', args: ['test', '--manifest-path', manifest, '--features', 'mainnet'] },
];

let failed = false;
for (const run of runs) {
  console.log('\n=== Tests ' + run.label + ' ===');
  const r = spawnSync(cargo, run.args, { stdio: 'inherit', shell: false });
  if (r.status !== 0) failed = true;
}

process.exit(failed ? 1 : 0);
