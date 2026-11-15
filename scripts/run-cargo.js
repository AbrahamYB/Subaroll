const { spawnSync } = require('node:child_process');
const os = require('node:os');
const path = require('node:path');

const cargo = process.platform === 'win32'
  ? path.join(process.env.USERPROFILE || '', '.cargo', 'bin', 'cargo.exe')
  : path.join(os.homedir(), '.cargo', 'bin', 'cargo');
const result = spawnSync(cargo, process.argv.slice(2), { stdio: 'inherit', env: process.env });
process.exit(result.status ?? 1);
