const { spawnSync } = require('node:child_process');

const result = spawnSync(process.execPath, [
  '--test',
  '--test-isolation=none',
  '--test-force-exit',
  'test/burner.test.js',
  'test/compressor.test.js',
  'test/release.test.js',
], {
  cwd: process.cwd(),
  encoding: 'utf8',
  windowsHide: true,
});

process.stdout.write(result.stdout || '');
process.stderr.write(result.stderr || '');

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

if (result.status === 0) process.exit(0);

const report = `${result.stdout || ''}\n${result.stderr || ''}`;
const passed = report.match(/(?:ℹ|#)\s*pass\s+(\d+)/i);
const failed = report.match(/(?:ℹ|#)\s*fail\s+(\d+)/i);
const cancelled = report.match(/(?:ℹ|#)\s*cancelled\s+(\d+)/i);

// GitHub's macOS ARM runner can return 1 after Node has reported a completely
// successful suite. Accept only an explicit, non-empty, zero-failure summary.
if (passed && Number(passed[1]) > 0 && failed?.[1] === '0' && cancelled?.[1] === '0') {
  console.warn('Node returned a stale non-zero status after a zero-failure test summary; continuing.');
  process.exit(0);
}

process.exit(result.status ?? 1);
