const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { compressionPlan, videoFilters } = require('../lib/compressor');

test('compression plan reserves overhead and audio under the target size', () => {
  const plan = compressionPlan({ duration: 60, width: 1920, height: 1080 }, 25);
  assert.ok(plan.videoBitrate > 3_000_000);
  assert.equal(plan.audioBitrate, 128_000);
  assert.equal(plan.outputHeight, 1080);
  assert.ok((plan.totalBitrate * 60 / 8) < 25 * 1024 * 1024);
});

test('a usable 1080p bitrate does not trigger a premature downscale', () => {
  const plan = compressionPlan({ duration: 10, width: 1920, height: 1080, fps: '30/1' }, 3);
  assert.ok(plan.videoBitrate > 2_000_000);
  assert.equal(plan.outputHeight, 1080);
  assert.equal(plan.outputFps, 30);
  assert.doesNotMatch(videoFilters({ isHdr: false, height: 1080 }, plan), /scale=/);
  assert.doesNotMatch(videoFilters({ isHdr: false, height: 1080 }, plan), /fps=/);
});

test('60 fps is preserved through 720p and reduced only when that budget is impossible', () => {
  const preserve = compressionPlan({ duration: 10, width: 1920, height: 1080, fps: '60/1' }, 3);
  assert.equal(preserve.outputHeight, 720);
  assert.equal(preserve.outputFps, 60);
  assert.doesNotMatch(videoFilters({ isHdr: false, height: 1080, fps: '60/1' }, preserve), /fps=/);

  const constrained = compressionPlan({ duration: 10, width: 1920, height: 1080, fps: '60/1' }, 1.5);
  assert.equal(constrained.outputHeight, 720);
  assert.equal(constrained.outputFps, 30);
  assert.match(videoFilters({ isHdr: false, height: 1080, fps: '60/1' }, constrained), /fps=30/);
});

test('low bitrate targets automatically lower resolution without upscaling', () => {
  assert.equal(compressionPlan({ duration: 600, width: 1920, height: 1080 }, 8).outputHeight, 480);
  assert.equal(compressionPlan({ duration: 60, width: 854, height: 480 }, 100).outputHeight, 480);
});

test('HDR compression tone maps to Discord-compatible SDR', () => {
  const filter = videoFilters({ isHdr: true, height: 2160 }, { outputHeight: 1080 });
  assert.match(filter, /tonemap=hable/);
  assert.match(filter, /scale=-2:1080:flags=lanczos/);
  assert.match(filter, /format=yuv420p$/);
});

test('compression mode is integrated into the native app UI', () => {
  const html = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'index.html'), 'utf8');
  const renderer = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'renderer.js'), 'utf8');
  const css = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'app.css'), 'utf8');
  assert.match(html, /id="compressMode"/);
  assert.match(html, /id="targetSize"/);
  assert.match(html, /id="compressionCodec"/);
  assert.match(html, /Auto · prefer hardware AV1/);
  assert.match(html, /id="targetDown"[\s\S]*id="targetUp"/);
  assert.match(html, /class="target-row"/);
  assert.match(html, /id="videoPicker"[\s\S]*id="compressionOptions"[\s\S]*class="subtitle-only">Subtitles/);
  assert.match(renderer, /startCompression/);
  assert.match(renderer, /function stepTargetSize/);
  assert.match(renderer, /state\.mode === 'compress'/);
  assert.doesNotMatch(html, /outputPicker|Choose destination/);
  assert.match(renderer, /The compressed file saves beside the source automatically/);
  assert.match(renderer, /codecPreference/);
  assert.match(renderer, /availableAv1Encoders/);
  assert.match(css, /\.target-row > small[^}]*font-weight: 500/);
});

test('Windows compression auto-detects hardware and reports the encoder used', () => {
  const backend = fs.readFileSync(path.join(__dirname, '..', 'src-tauri', 'src', 'media.rs'), 'utf8');
  const renderer = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'renderer.js'), 'utf8');
  assert.match(backend, /for encoder in \["h264_nvenc", "h264_qsv", "h264_amf"\]/);
  assert.match(backend, /for encoder in \["av1_nvenc", "av1_qsv", "av1_amf"\]/);
  assert.match(backend, /compression_recipe_usable/);
  assert.match(backend, /name: "libx264"\.into\(\)/);
  assert.match(backend, /"-preset", "p5"/);
  assert.match(backend, /"-multipass",\s*"qres"/);
  assert.match(backend, /"constqp"/);
  assert.match(backend, /compression_hardware_recipe_with_qp/);
  assert.match(renderer, /NVIDIA NVENC/);
  assert.match(renderer, /CPU x264 two-pass/);
});

test('window has a drag region and viewport-locked responsive layout', () => {
  const html = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'index.html'), 'utf8');
  const css = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'app.css'), 'utf8');
  assert.match(html, /class="window-drag-region" data-tauri-drag-region/);
  assert.match(css, /-webkit-app-region: drag/);
  assert.match(css, /html, body \{[^}]*overflow: hidden/);
  assert.doesNotMatch(css, /@media \(max-height:/);
});

test('window remains stable while zoom stays locked at a stable 100%', () => {
  const config = fs.readFileSync(path.join(__dirname, '..', 'src-tauri', 'tauri.conf.json'), 'utf8');
  const renderer = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'renderer.js'), 'utf8');
  const updater = fs.readFileSync(path.join(__dirname, '..', 'scripts', 'update-installed-mac.sh'), 'utf8');
  assert.match(config, /"height": 760/);
  assert.match(config, /"minHeight": 720/);
  assert.doesNotMatch(config, /zoom/i);
  assert.match(renderer, /function fitWindowToContent/);
  assert.doesNotMatch(renderer, /api\.fitWindow/);
  assert.doesNotMatch(renderer, /ResizeObserver/);
  assert.match(updater, /npm run build:mac/);
  assert.match(updater, /rm -rf "\$installed_app"/);
});

test('macOS app permits only one main instance', () => {
  const backend = fs.readFileSync(path.join(__dirname, '..', 'src-tauri', 'src', 'lib.rs'), 'utf8');
  const pkg = fs.readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8');
  assert.match(backend, /tauri_plugin_single_instance/);
  assert.doesNotMatch(pkg, /electron/i);
});
