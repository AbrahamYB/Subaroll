const { spawnSync } = require('node:child_process');
const os = require('node:os');
const path = require('node:path');

const cargoBin = process.platform === 'win32'
  ? path.join(process.env.USERPROFILE || '', '.cargo', 'bin')
  : path.join(os.homedir(), '.cargo', 'bin');
const binary = process.platform === 'win32'
  ? path.join(__dirname, '..', 'node_modules', '.bin', 'tauri.cmd')
  : path.join(__dirname, '..', 'node_modules', '.bin', 'tauri');
const env = { ...process.env, PATH: `${cargoBin}${path.delimiter}${process.env.PATH || ''}` };
const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit', env });
process.exit(result.status ?? 1);
