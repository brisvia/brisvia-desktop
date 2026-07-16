// Compiles the TWO E2E test binaries and leaves them with a stable name inside target/debug
// (where their neighboring DLLs live). They are never published: they are only for real testing.
//
//   brisvia-miner-e2e.exe          -> e2e feature (test network / tprv keys), redirected to regtest via env.
//   brisvia-miner-mainnet-e2e.exe  -> mainnet + e2e feature (for the wait-mode journey).
//
// Locates cargo even if it is not on PATH (rustup in ~/.cargo/bin), just like run-rust-tests.js.
'use strict';

const { spawnSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

function findCargo() {
  const probe = spawnSync('cargo', ['--version'], { encoding: 'utf8', shell: false });
  if (probe.status === 0) return 'cargo';
  const exe = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
  const candidate = path.join(os.homedir(), '.cargo', 'bin', exe);
  if (fs.existsSync(candidate)) return candidate;
  return null;
}

const cargo = findCargo();
if (!cargo) {
  console.error('cargo (Rust) not found. Install Rust with rustup.');
  process.exit(1);
}

const manifest = path.resolve(__dirname, '..', '..', 'src-tauri', 'Cargo.toml');
const targetDir = path.resolve(__dirname, '..', '..', 'src-tauri', 'target', 'debug');
const exe = process.platform === 'win32' ? '.exe' : '';
const base = path.join(targetDir, `brisvia-miner${exe}`);

const builds = [
  { features: 'e2e', out: `brisvia-miner-e2e${exe}` },
  { features: 'mainnet,e2e', out: `brisvia-miner-mainnet-e2e${exe}` },
];

for (const b of builds) {
  console.log(`\n=== Building features: ${b.features} ===`);
  const r = spawnSync(cargo, ['build', '--manifest-path', manifest, '--features', b.features], {
    stdio: 'inherit',
    shell: false,
  });
  if (r.status !== 0) {
    console.error(`Build failed with features ${b.features}`);
    process.exit(1);
  }
  const dest = path.join(targetDir, b.out);
  fs.copyFileSync(base, dest);
  console.log(`OK -> ${dest}`);
}
console.log('\nE2E binaries ready.');
