const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const cargoName = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
const candidates = [
  process.env.CARGO,
  process.env.CARGO_HOME && path.join(process.env.CARGO_HOME, 'bin', cargoName),
  path.join(os.homedir(), '.cargo', 'bin', cargoName),
].filter(Boolean);
const cargo = candidates.find(candidate => fs.existsSync(candidate)) || 'cargo';
const result = spawnSync(cargo, process.argv.slice(2), { stdio: 'inherit', env: process.env });
if (result.error) console.error(`Unable to launch Cargo: ${result.error.message}`);
process.exit(result.status ?? 1);
