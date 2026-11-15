const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const packageJson = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));

test('release workflow builds only Apple-silicon macOS and Windows 11 x64', () => {
  const workflow = fs.readFileSync(path.join(root, '.github', 'workflows', 'release.yml'), 'utf8');
  assert.match(workflow, /aarch64-apple-darwin/);
  assert.doesNotMatch(workflow, /x86_64-apple-darwin/);
  assert.match(workflow, /windows-2022/);
  assert.match(workflow, /x86_64-pc-windows-msvc/);
  assert.match(workflow, /--bundles dmg/);
  assert.match(workflow, /--bundles nsis,msi/);
  assert.match(workflow, /node-version: 26/);
  assert.match(packageJson.scripts.test, /run-node-tests\.js/);
  const nodeTestRunner = fs.readFileSync(path.join(root, 'scripts', 'run-node-tests.js'), 'utf8');
  assert.match(nodeTestRunner, /failed\?\.\[1\] === '0'/);
  assert.match(nodeTestRunner, /cancelled\?\.\[1\] === '0'/);
});

test('Windows release bundles a checksum-pinned FFmpeg with libass', () => {
  const config = fs.readFileSync(path.join(root, 'src-tauri', 'tauri.release.windows.json'), 'utf8');
  const preparation = fs.readFileSync(path.join(root, 'scripts', 'prepare-windows-ffmpeg.ps1'), 'utf8');
  const backend = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'media.rs'), 'utf8');
  assert.match(config, /binaries\/ffmpeg/);
  assert.match(config, /binaries\/ffprobe/);
  assert.match(preparation, /Get-FileHash[^\n]*SHA256/);
  assert.match(preparation, /subtitles\\s\+V->V/);
  assert.match(backend, /current_exe\(\)[\s\S]*join\("ffmpeg\.exe"\)/);
});
