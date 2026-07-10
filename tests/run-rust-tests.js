// Runner de los tests del backend (Rust). Corre la suite en las DOS builds de red:
//   - testnet (por defecto): el guard del wpkh exige llaves tprv.
//   - mainnet (--features mainnet): el guard del wpkh exige llaves xprv.
// Así, el test `wallet_key_tests::ext_key_prefix_matches_build_network` cuida el bug de la llave
// ("wpkh(): key '...' is not valid") en las dos redes, con el generador de descriptores REAL.
//
// Localiza cargo aunque no esté en el PATH (típico en esta máquina: rustup en ~/.cargo/bin).
'use strict';

const { spawnSync } = require('child_process');
const path = require('path');
const os = require('os');
const fs = require('fs');

function findCargo() {
  // 1) cargo en el PATH.
  const probe = spawnSync('cargo', ['--version'], { encoding: 'utf8', shell: false });
  if (probe.status === 0) return 'cargo';
  // 2) Fallback: instalación de rustup en el home del usuario.
  const exe = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
  const candidate = path.join(os.homedir(), '.cargo', 'bin', exe);
  if (fs.existsSync(candidate)) return candidate;
  return null;
}

const cargo = findCargo();
if (!cargo) {
  console.error('No se encontró cargo (Rust). Instalá Rust con rustup para correr los tests del backend.');
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
