const path = require('node:path');
const os = require('node:os');
const fsp = require('node:fs/promises');
const { spawn } = require('node:child_process');

function clamp(value, min, max) { return Math.max(min, Math.min(max, value)); }

function frameRate(value) {
  const [numerator, denominator = '1'] = String(value || '30').split('/');
  const result = Number(numerator) / Math.max(1, Number(denominator));
  return Number.isFinite(result) && result > 0 ? result : 30;
}

function heightForBitrate(videoBitrate) {
  if (videoBitrate >= 2_000_000) return 1080;
  if (videoBitrate >= 1_200_000) return 900;
  if (videoBitrate >= 650_000) return 720;
  if (videoBitrate >= 350_000) return 540;
  return 480;
}

function compressionPlan(metadata, targetSizeMB) {
  const duration = Math.max(0.1, Number(metadata.duration) || 0.1);
  const targetBytes = clamp(Number(targetSizeMB) || 25, 1, 10000) * 1024 * 1024;
  const totalBitrate = Math.floor((targetBytes * 8 / duration) * 0.96);
  const audioBitrate = totalBitrate >= 1_500_000 ? 128_000 : totalBitrate >= 700_000 ? 96_000 : 64_000;
  const videoBitrate = Math.max(120_000, totalBitrate - audioBitrate);
  const sourceFps = frameRate(metadata.fps);
  const rateAtSourceFps = Math.floor(videoBitrate * Math.min(1, 30 / sourceFps));
  let maxHeight = heightForBitrate(rateAtSourceFps);
  let outputFps = sourceFps;
  if (sourceFps > 30 && maxHeight < 720) {
    outputFps = Math.min(30, sourceFps / 2);
    maxHeight = heightForBitrate(videoBitrate);
  }
  const outputHeight = Math.min(Number(metadata.height) || maxHeight, maxHeight);
  return { targetBytes, totalBitrate, videoBitrate, audioBitrate, outputHeight, outputFps };
}

function progressFromLine(line, duration) {
  const match = String(line).match(/^out_time_(?:ms|us)=(\d+)/m);
  return match && duration ? clamp(Number(match[1]) / 1_000_000 / duration, 0, 1) : null;
}

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { windowsHide: true });
    let stderr = '';
    let stdout = '';
    child.stdout.on('data', chunk => {
      stdout += chunk;
      options.onStdout?.(chunk.toString());
    });
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.on('error', reject);
    child.on('close', code => code === 0 ? resolve() : reject(Object.assign(new Error(stderr.trim().split(/\r?\n/).slice(-6).join('\n') || `FFmpeg exited with ${code}`), { code, stderr })));
    options.onSpawn?.(child);
  });
}

function videoFilters(metadata, plan) {
  const filters = [];
  if (metadata.isHdr) {
    filters.push('zscale=t=linear:npl=100');
    filters.push('format=gbrpf32le');
    filters.push('zscale=p=bt709');
    filters.push('tonemap=hable:desat=0');
    filters.push('zscale=t=bt709:m=bt709:r=tv');
  }
  if (plan.outputHeight < metadata.height) filters.push(`scale=-2:${plan.outputHeight}:flags=lanczos`);
  const sourceFps = frameRate(metadata.fps);
  if (plan.outputFps && plan.outputFps < sourceFps - 0.01) filters.push(`fps=${plan.outputFps}`);
  filters.push('format=yuv420p');
  return filters.join(',');
}

async function compressVideo(ffmpeg, ffprobe, probeMedia, input, handlers = {}) {
  const metadata = input.metadata || await probeMedia(ffprobe, input.videoPath);
  const plan = compressionPlan(metadata, input.targetSizeMB);
  const tempDir = await fsp.mkdtemp(path.join(os.tmpdir(), 'SubarollCompress-'));
  const passlog = path.join(tempDir, 'pass');
  const common = [
    '-hide_banner', '-y', '-noautorotate', '-i', input.videoPath,
    '-map', '0:v:0', '-vf', videoFilters(metadata, plan),
    '-c:v', 'libx264', '-preset', 'slow', '-b:v', String(plan.videoBitrate),
    '-maxrate', String(Math.round(plan.videoBitrate * 1.25)), '-bufsize', String(plan.videoBitrate * 2),
    '-passlogfile', passlog,
  ];
  let progressBuffer = '';
  try {
    await run(ffmpeg, [...common, '-pass', '1', '-an', '-progress', 'pipe:1', '-nostats', '-f', 'null', '-'], {
      onSpawn: handlers.onSpawn,
      onStdout: chunk => {
        progressBuffer += chunk;
        const lines = progressBuffer.split(/\r?\n/);
        progressBuffer = lines.pop() || '';
        for (const line of lines) {
          const value = progressFromLine(line, metadata.duration);
          if (value !== null) handlers.onProgress?.(value * 0.5);
        }
      },
    });
    progressBuffer = '';
    await run(ffmpeg, [
      ...common, '-pass', '2', '-map', '0:a:0?', '-c:a', 'aac', '-b:a', String(plan.audioBitrate),
      '-map_metadata', '0', '-map_chapters', '0', '-movflags', '+faststart',
      '-progress', 'pipe:1', '-nostats', input.outputPath,
    ], {
      onSpawn: handlers.onSpawn,
      onStdout: chunk => {
        progressBuffer += chunk;
        const lines = progressBuffer.split(/\r?\n/);
        progressBuffer = lines.pop() || '';
        for (const line of lines) {
          const value = progressFromLine(line, metadata.duration);
          if (value !== null) handlers.onProgress?.(0.5 + value * 0.5);
        }
      },
    });
    const outputMetadata = await probeMedia(ffprobe, input.outputPath);
    const stat = await fsp.stat(input.outputPath);
    if (stat.size > plan.targetBytes * 1.03) throw new Error(`Compressed file exceeded the ${input.targetSizeMB} MB target.`);
    handlers.onProgress?.(1);
    return { output: input.outputPath, bytes: stat.size, plan, metadata: outputMetadata };
  } finally {
    await fsp.rm(tempDir, { recursive: true, force: true });
  }
}

module.exports = { compressionPlan, videoFilters, compressVideo };
