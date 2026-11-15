const fs = require('node:fs');
const fsp = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { spawn, spawnSync } = require('node:child_process');

const VIDEO_EXTENSIONS = ['mp4', 'mkv', 'mov', 'avi', 'webm', 'm4v', 'ts', 'mts', 'm2ts'];
const SUBTITLE_EXTENSIONS = ['ass', 'ssa', 'srt', 'vtt', 'sub', 'smi'];

function executableName(name) {
  return process.platform === 'win32' ? `${name}.exe` : name;
}

function commandExists(command) {
  const result = spawnSync(command, ['-version'], { stdio: 'ignore', windowsHide: true });
  return !result.error && result.status === 0;
}

function supportsLibass(command) {
  const result = spawnSync(command, ['-hide_banner', '-filters'], {
    encoding: 'utf8',
    windowsHide: true,
    timeout: 10000,
  });
  return !result.error && result.status === 0 && /\bsubtitles\s+V->V\b/.test(result.stdout || '');
}

function supportsEncoder(command, encoderName) {
  const result = spawnSync(command, ['-hide_banner', '-encoders'], {
    encoding: 'utf8',
    windowsHide: true,
    timeout: 10000,
  });
  return !result.error && result.status === 0 && new RegExp(`\\b${encoderName}\\b`).test(result.stdout || '');
}

function findTools(customFfmpeg = '') {
  const ffmpegName = executableName('ffmpeg');
  const ffprobeName = executableName('ffprobe');
  const candidates = [
    customFfmpeg,
    process.env.SUBAROLL_FFMPEG,
    process.env.SUBTITLE_BURNER_FFMPEG,
    process.resourcesPath && path.join(process.resourcesPath, 'bin', process.platform, process.arch, ffmpegName),
    process.platform === 'darwin' && '/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg',
    process.platform === 'darwin' && '/usr/local/opt/ffmpeg-full/bin/ffmpeg',
    process.platform === 'darwin' && '/opt/homebrew/bin/ffmpeg',
    process.platform === 'darwin' && '/usr/local/bin/ffmpeg',
    ffmpegName,
  ].filter(Boolean);

  for (const candidate of [...new Set(candidates)]) {
    if (!commandExists(candidate) || !supportsLibass(candidate)) continue;
    const siblingProbe = path.join(path.dirname(candidate), ffprobeName);
    const ffprobe = commandExists(siblingProbe) ? siblingProbe : (commandExists(ffprobeName) ? ffprobeName : '');
    if (ffprobe) {
      const videoEncoder = process.platform === 'darwin' && supportsEncoder(candidate, 'h264_videotoolbox')
        ? 'h264_videotoolbox'
        : 'libx264';
      return { ffmpeg: candidate, ffprobe, videoEncoder };
    }
  }
  return null;
}

function colorMetadataArgs(metadata = {}) {
  const pairs = [
    ['-color_range', metadata.colorRange],
    ['-colorspace', metadata.colorSpace],
    ['-color_trc', metadata.colorTransfer],
    ['-color_primaries', metadata.colorPrimaries],
  ];
  return pairs.flatMap(([flag, value]) => value && value !== 'unknown' ? [flag, value] : []);
}

function sourceAwareBitrateArgs(metadata = {}, quality = 90) {
  const sourceBitrate = Number(metadata.bitRate || 0);
  if (!sourceBitrate) return [];
  // Quality 90 matches the source bitrate. Lower values trade size for quality;
  // higher values provide headroom without allowing an unbounded CQ encode.
  const factor = 0.55 + ((quality - 60) / 40) * 0.6;
  const target = Math.round(sourceBitrate * factor);
  return ['-b:v', String(target), '-maxrate', String(Math.round(target * 1.35)), '-bufsize', String(target * 2)];
}

function videoEncoderArgs(ffmpeg, platform = process.platform, requestedQuality = 90, metadata = {}) {
  const quality = Math.round(Math.max(60, Math.min(100, assNumber(requestedQuality, 90))));
  const colorArgs = colorMetadataArgs(metadata);
  // VideoToolbox's bitrate mode is content-dependent even with constant_bit_rate
  // enabled and can undershoot a 4K HDR source by more than 50%. Its quality
  // mode is consistent; map the friendly 60–100 UI range to the useful VT range.
  // The UI is explicitly labelled CQ, so pass that value through to
  // VideoToolbox. The previous hidden remapping made CQ 90 only q78 and could
  // still discard visible texture and gradients.
  const hardwareQuality = quality;
  const rateControlArgs = ['-q:v', String(hardwareQuality)];
  const needsTenBit = metadata.isHdr || Number(metadata.bitDepth || 0) > 8;
  if (platform === 'darwin' && supportsEncoder(ffmpeg, 'hevc_videotoolbox')) {
    return {
      name: 'hevc_videotoolbox',
      args: [
        '-c:v', 'hevc_videotoolbox', '-profile:v', needsTenBit ? 'main10' : 'main', ...rateControlArgs,
        '-allow_sw', '0', '-realtime', '0', '-prio_speed', '0', '-spatial_aq', '1',
        '-pix_fmt', needsTenBit ? 'p010le' : 'yuv420p', '-tag:v', 'hvc1', ...colorArgs,
      ],
      quality,
      hdr: Boolean(metadata.isHdr),
      tenBit: needsTenBit,
    };
  }
  if (needsTenBit && supportsEncoder(ffmpeg, 'libx265')) {
    const crf = Math.round(41 - quality * 0.3);
    return {
      name: 'libx265',
      args: ['-c:v', 'libx265', '-preset', 'medium', '-crf', String(crf), '-pix_fmt', 'yuv420p10le', ...colorArgs],
      quality,
      hdr: Boolean(metadata.isHdr),
      tenBit: true,
    };
  }
  if (platform === 'darwin' && supportsEncoder(ffmpeg, 'h264_videotoolbox')) {
    return {
      name: 'h264_videotoolbox',
      args: [
        '-c:v', 'h264_videotoolbox', '-profile:v', 'high', ...rateControlArgs,
        '-allow_sw', '0', '-realtime', '0', '-prio_speed', '0', '-spatial_aq', '1',
        '-coder', 'cabac', '-pix_fmt', 'yuv420p', ...colorArgs,
      ], quality,
    };
  }
  const crf = Math.round(41 - quality * 0.3);
  return {
    name: 'libx264',
    args: ['-c:v', 'libx264', '-preset', 'medium', '-crf', String(crf), '-pix_fmt', 'yuv420p'],
    quality,
  };
}

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { windowsHide: true, ...options });
    let stdout = '';
    let stderr = '';
    child.stdout?.on('data', chunk => { stdout += chunk.toString(); options.onStdout?.(chunk.toString()); });
    child.stderr?.on('data', chunk => { stderr += chunk.toString(); options.onStderr?.(chunk.toString()); });
    child.on('error', reject);
    child.on('close', code => {
      if (code === 0) resolve({ stdout, stderr, child });
      else reject(Object.assign(new Error(lastUsefulLine(stderr) || `Process exited with code ${code}`), { code, stderr }));
    });
    options.onSpawn?.(child);
  });
}

function lastUsefulLine(text) {
  const lines = String(text).split(/\r?\n/).map(line => line.trim()).filter(Boolean);
  return lines.slice(-6).join('\n');
}

async function probeMedia(ffprobe, videoPath) {
  const { stdout } = await run(ffprobe, [
    '-v', 'error', '-print_format', 'json', '-show_format', '-show_streams', videoPath,
  ]);
  const data = JSON.parse(stdout);
  const video = data.streams?.find(stream => stream.codec_type === 'video');
  if (!video) throw new Error('The selected file does not contain a video stream.');
  const duration = Number(video.duration || data.format?.duration || 0);
  const sampleAspect = parseRatio(video.sample_aspect_ratio, 1);
  const pixelFormat = video.pix_fmt || '';
  const bitDepth = Number(video.bits_per_raw_sample) || (/10|p010/i.test(pixelFormat) ? 10 : /12/i.test(pixelFormat) ? 12 : 8);
  const colorTransfer = video.color_transfer || '';
  const isHdr = /smpte2084|arib-std-b67/i.test(colorTransfer) || (bitDepth > 8 && video.color_primaries === 'bt2020');
  const dolbyVision = Boolean(video.side_data_list?.some(item => /dovi|dolby vision/i.test(item.side_data_type || '')));
  const audioStreams = (data.streams || []).filter(stream => stream.codec_type === 'audio').map(stream => ({
    codec: stream.codec_name || '', language: stream.tags?.language || '', channels: Number(stream.channels || 0),
    bitRate: Number(stream.bit_rate || 0), profile: stream.profile || '',
  }));
  return {
    width: Number(video.width || 0),
    height: Number(video.height || 0),
    duration: Number.isFinite(duration) ? duration : 0,
    codec: video.codec_name || 'unknown',
    bitRate: Number(video.bit_rate || 0),
    fps: video.avg_frame_rate || video.r_frame_rate || '',
    sampleAspectRatio: video.sample_aspect_ratio || '1:1',
    displayAspectRatio: video.display_aspect_ratio || '',
    displayAspect: parseRatio(video.display_aspect_ratio, (Number(video.width) / Number(video.height)) * sampleAspect),
    rotation: Number(video.side_data_list?.find(item => Number.isFinite(Number(item.rotation)))?.rotation || video.tags?.rotate || 0),
    pixelFormat,
    bitDepth,
    colorRange: video.color_range || '',
    colorSpace: video.color_space || '',
    colorTransfer,
    colorPrimaries: video.color_primaries || '',
    isHdr,
    dolbyVision,
    audioStreams,
  };
}

function assNumber(value, fallback = 0) {
  const number = Number.parseFloat(value);
  return Number.isFinite(number) ? number : fallback;
}

function splitAssRow(row, fieldCount) {
  const parts = [];
  let rest = row;
  for (let index = 0; index < fieldCount - 1; index += 1) {
    const comma = rest.indexOf(',');
    if (comma === -1) break;
    parts.push(rest.slice(0, comma));
    rest = rest.slice(comma + 1);
  }
  parts.push(rest);
  return parts;
}

function formatNumber(value) {
  return Number(value.toFixed(2)).toString();
}

function parseFrameRate(value) {
  const match = String(value || '').match(/^(\d+(?:\.\d+)?)\/(\d+(?:\.\d+)?)$/);
  if (match && Number(match[2])) return Number(match[1]) / Number(match[2]);
  const number = Number(value);
  return Number.isFinite(number) && number > 0 ? number : 25;
}

function parseRatio(value, fallback = 0) {
  const match = String(value || '').match(/^(\d+(?:\.\d+)?)[/:](\d+(?:\.\d+)?)$/);
  if (!match || !Number(match[2])) return fallback;
  return Number(match[1]) / Number(match[2]);
}

function automaticCanvas(metadata = {}) {
  const width = Math.max(0, Math.round(assNumber(metadata.width, 0)));
  const height = Math.max(0, Math.round(assNumber(metadata.height, 0)));
  if (!width || !height || width / height <= (16 / 9) + 0.01) {
    return { width, height, bottomPadding: 0, padded: false };
  }
  // Cinematic sources often store only the active picture. Add the missing
  // 16:9 area below it so subtitles can live there without moving/stretching
  // the picture itself. Keep dimensions even for hardware encoders.
  const targetHeight = Math.ceil((width * 9 / 16) / 2) * 2;
  return { width, height: targetHeight, bottomPadding: targetHeight - height, padded: true };
}

async function inspectSubtitleTiming(subtitlePath) {
  const text = await fsp.readFile(subtitlePath, 'utf8');
  const timestamp = /(\d{1,2}):(\d{2}):(\d{2})[,.](\d{2,3})/g;
  let match;
  let cueCount = 0;
  let lastCueSeconds = 0;
  while ((match = timestamp.exec(text))) {
    cueCount += 1;
    const fraction = Number(match[4]) / (match[4].length === 2 ? 100 : 1000);
    lastCueSeconds = Math.max(lastCueSeconds, Number(match[1]) * 3600 + Number(match[2]) * 60 + Number(match[3]) + fraction);
  }
  return { cueCount: Math.floor(cueCount / 2), lastCueSeconds };
}

function previewTimestamp(metadata, requestedTime) {
  const duration = Math.max(0, assNumber(metadata?.duration, 0));
  const fps = parseFrameRate(metadata?.fps);
  // Containers commonly report their end timestamp just after the final video
  // frame. Stay safely inside the stream so FFmpeg always has a frame to emit.
  const finalFrameTime = Math.max(0, duration - Math.max(0.1, 2 / fps));
  return Math.max(0, Math.min(finalFrameTime, assNumber(requestedTime, 0)));
}

function transformAss(source, options = {}) {
  const scale = Math.max(0.25, Math.min(3, assNumber(options.scalePercent, 100) / 100));
  const position = Math.max(0, Math.min(100, assNumber(options.positionPercent, 95)));
  const plain = Boolean(options.plain);
  const targetAspect = assNumber(options.displayAspect, options.width && options.height ? options.width / options.height : 16 / 9);
  let text = String(source).replace(/^\uFEFF/, '').replace(/\r\n/g, '\n');
  let playResY = assNumber(text.match(/^PlayResY:\s*([\d.]+)/mi)?.[1], 288);
  let playResX = assNumber(text.match(/^PlayResX:\s*([\d.]+)/mi)?.[1], Math.round(playResY * targetAspect));

  if (plain) {
    playResX = Math.round(360 * targetAspect);
    text = text.replace(/^PlayResX:.*$/mi, `PlayResX: ${playResX}`);
    text = text.replace(/^PlayResY:.*$/mi, 'PlayResY: 360');
    if (!/^PlayResX:/mi.test(text)) text = text.replace(/^\[Script Info\]\s*$/mi, `[Script Info]\nPlayResX: ${playResX}`);
    if (!/^PlayResY:/mi.test(text)) text = text.replace(/^\[Script Info\]\s*$/mi, '[Script Info]\nPlayResY: 360');
    if (!/^ScaledBorderAndShadow:/mi.test(text)) text = text.replace(/^\[Script Info\]\s*$/mi, '[Script Info]\nScaledBorderAndShadow: yes');
    playResY = 360;
  }

  const lines = text.split('\n');
  let section = '';
  let styleFormat = [];
  let eventFormat = [];
  const dialogueStyleNames = new Set();

  for (const line of lines) {
    const header = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (header) section = header[1].toLowerCase();
    if (section === 'events' && /^Format:/i.test(line)) {
      eventFormat = line.slice(line.indexOf(':') + 1).split(',').map(value => value.trim().toLowerCase());
    } else if (section === 'events' && /^Dialogue:/i.test(line) && eventFormat.length) {
      const fields = splitAssRow(line.slice(line.indexOf(':') + 1), eventFormat.length);
      const style = fields[eventFormat.indexOf('style')]?.trim();
      const body = fields[eventFormat.indexOf('text')] || '';
      if (style && !/\\(?:pos|move|an[4-9])\b/i.test(body)) dialogueStyleNames.add(style);
    }
  }

  section = '';
  return lines.map(line => {
    const header = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (header) section = header[1].toLowerCase();
    const isStyles = section === 'v4+ styles' || section === 'v4 styles';
    if (isStyles && /^Format:/i.test(line)) {
      styleFormat = line.slice(line.indexOf(':') + 1).split(',').map(value => value.trim().toLowerCase());
      return line;
    }
    if (isStyles && /^Style:/i.test(line) && styleFormat.length) {
      const prefix = line.slice(0, line.indexOf(':') + 1);
      const fields = splitAssRow(line.slice(line.indexOf(':') + 1), styleFormat.length);
      const nameIndex = styleFormat.indexOf('name');
      const fontIndex = styleFormat.indexOf('fontname');
      const sizeIndex = styleFormat.indexOf('fontsize');
      const marginVIndex = styleFormat.indexOf('marginv');
      const primaryIndex = styleFormat.indexOf('primarycolour');
      const outlineColorIndex = styleFormat.indexOf('outlinecolour');
      const backColorIndex = styleFormat.indexOf('backcolour');
      const boldIndex = styleFormat.indexOf('bold');
      const borderStyleIndex = styleFormat.indexOf('borderstyle');
      const outlineIndex = styleFormat.indexOf('outline');
      const shadowIndex = styleFormat.indexOf('shadow');
      const alignmentIndex = styleFormat.indexOf('alignment');
      const marginLIndex = styleFormat.indexOf('marginl');
      const marginRIndex = styleFormat.indexOf('marginr');
      const styleName = fields[nameIndex]?.trim();
      const shouldAdjust = plain || dialogueStyleNames.size === 0 || dialogueStyleNames.has(styleName);

      if (plain) {
        if (fontIndex >= 0) fields[fontIndex] = 'Trebuchet MS';
        if (sizeIndex >= 0) fields[sizeIndex] = formatNumber(24 * scale);
        if (primaryIndex >= 0) fields[primaryIndex] = '&H00FFFFFF';
        if (outlineColorIndex >= 0) fields[outlineColorIndex] = '&H00000000';
        if (backColorIndex >= 0) fields[backColorIndex] = '&H00000000';
        if (boldIndex >= 0) fields[boldIndex] = '0';
        if (borderStyleIndex >= 0) fields[borderStyleIndex] = '1';
        if (outlineIndex >= 0) fields[outlineIndex] = '2';
        if (shadowIndex >= 0) fields[shadowIndex] = '1';
        if (alignmentIndex >= 0) fields[alignmentIndex] = '2';
        if (marginLIndex >= 0) fields[marginLIndex] = '10';
        if (marginRIndex >= 0) fields[marginRIndex] = '10';
        if (marginVIndex >= 0) fields[marginVIndex] = '18';
      } else if (shouldAdjust) {
        if (sizeIndex >= 0) fields[sizeIndex] = formatNumber(assNumber(fields[sizeIndex], 55) * scale);
      }
      return `${prefix}${fields.join(',')}`;
    }
    if (section === 'events' && /^Dialogue:/i.test(line) && eventFormat.length) {
      const prefix = line.slice(0, line.indexOf(':') + 1);
      const fields = splitAssRow(line.slice(line.indexOf(':') + 1), eventFormat.length);
      const style = fields[eventFormat.indexOf('style')]?.trim();
      const textIndex = eventFormat.indexOf('text');
      if (dialogueStyleNames.has(style) && textIndex >= 0) {
        const scaledText = fields[textIndex].replace(/\\fs([\d.]+)/gi, (_, size) => `\\fs${formatNumber(assNumber(size) * scale)}`);
        const lineCount = Math.max(1, scaledText.split(/\\N/gi).length);
        const topRoom = Math.min(playResY * 0.3, 4 + 30 * scale * lineCount);
        const absoluteY = Math.round(topRoom + (playResY - 4 - topRoom) * position / 100);
        const absoluteAlignment = 2;
        fields[textIndex] = `{\\an${absoluteAlignment}\\pos(${Math.round(playResX / 2)},${absoluteY})}${scaledText.replace(/\\an[1-9]/gi, '')}`;
      }
      return `${prefix}${fields.join(',')}`;
    }
    return line;
  }).join('\n');
}

async function copyNearbyFonts(subtitlePath, fontsDir) {
  await fsp.mkdir(fontsDir, { recursive: true });
  const entries = await fsp.readdir(path.dirname(subtitlePath), { withFileTypes: true });
  const fonts = entries.filter(entry => entry.isFile() && /\.(ttf|otf|ttc|otc)$/i.test(entry.name));
  await Promise.all(fonts.map(font => fsp.copyFile(path.join(path.dirname(subtitlePath), font.name), path.join(fontsDir, font.name))));
  return fonts.length;
}

async function prepareSubtitle(ffmpeg, subtitlePath, metadata, settings) {
  const tempDir = await fsp.mkdtemp(path.join(os.tmpdir(), 'Subaroll-'));
  const extension = path.extname(subtitlePath).slice(1).toLowerCase();
  const converted = path.join(tempDir, 'source.ass');
  const prepared = path.join(tempDir, 'prepared.ass');
  const isStyled = extension === 'ass' || extension === 'ssa';

  if (extension === 'ass') {
    await fsp.copyFile(subtitlePath, converted);
  } else {
    await run(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-y', '-i', subtitlePath, converted]);
  }
  const source = await fsp.readFile(converted, 'utf8');
  const canvas = automaticCanvas(metadata);
  const result = transformAss(source, {
    scalePercent: settings.scalePercent,
    positionPercent: settings.positionPercent,
    plain: !isStyled,
    width: canvas.width,
    height: canvas.height,
    displayAspect: canvas.width / canvas.height,
  });
  await fsp.writeFile(prepared, result, 'utf8');
  const fontsDir = path.join(tempDir, 'fonts');
  const fontCount = await copyNearbyFonts(subtitlePath, fontsDir);
  return { tempDir, prepared, fontsDir, fontCount, assText: result };
}

function assTimestampSeconds(value) {
  const match = String(value || '').trim().match(/^(\d+):(\d{2}):(\d{2})(?:[.](\d{1,3}))?$/);
  if (!match) return 0;
  const fraction = Number(`0.${match[4] || 0}`);
  return Number(match[1]) * 3600 + Number(match[2]) * 60 + Number(match[3]) + fraction;
}

function activeSubtitleLineCount(assText, time) {
  const lines = String(assText).split(/\r?\n/);
  let section = '';
  let format = [];
  let maximum = 1;
  for (const line of lines) {
    const header = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (header) section = header[1].toLowerCase();
    if (section !== 'events') continue;
    if (/^Format:/i.test(line)) format = line.slice(line.indexOf(':') + 1).split(',').map(item => item.trim().toLowerCase());
    if (!/^Dialogue:/i.test(line) || !format.length) continue;
    const fields = splitAssRow(line.slice(line.indexOf(':') + 1), format.length);
    const start = assTimestampSeconds(fields[format.indexOf('start')]);
    const end = assTimestampSeconds(fields[format.indexOf('end')]);
    if (time >= start && time < end) maximum = Math.max(maximum, (fields[format.indexOf('text')]?.match(/\\N/gi) || []).length + 1);
  }
  return maximum;
}

function escapeFilterPath(filePath) {
  return String(filePath)
    .replace(/\\/g, '/')
    .replace(/'/g, "\\'")
    .replace(/:/g, '\\:')
    .replace(/,/g, '\\,')
    .replace(/\[/g, '\\[')
    .replace(/\]/g, '\\]');
}

function subtitleFilter(preparedPath, fontsDir, delaySeconds = 0, previewTime = null, processAlpha = false, timelineOffset = 0) {
  const delay = Math.max(-60, Math.min(60, assNumber(delaySeconds, 0)));
  const subs = `subtitles=filename='${escapeFilterPath(preparedPath)}':fontsdir='${escapeFilterPath(fontsDir)}'${processAlpha ? ':alpha=1' : ''}`;
  if (previewTime !== null) {
    const absoluteTime = Math.max(0, assNumber(previewTime, 0)) - delay;
    return `setpts=PTS+${formatNumber(absoluteTime)}/TB,${subs}`;
  }
  const offset = Math.max(0, assNumber(timelineOffset, 0)) - delay;
  const shiftedPts = value => `PTS${value < 0 ? '-' : '+'}${formatNumber(Math.abs(value))}/TB`;
  if (offset !== 0) return `setpts=${shiftedPts(offset)},${subs},setpts=${shiftedPts(-offset)}`;
  if (delay === 0) return subs;
  return `setpts=PTS-${formatNumber(delay)}/TB,${subs},setpts=PTS+${formatNumber(delay)}/TB`;
}

function exportVideoFilter(metadata, canvas, subtitleBurn) {
  const filters = [];
  if (canvas.padded) filters.push(`pad=${canvas.width}:${canvas.height}:0:0:black`);
  // Keep HDR in a 10-bit YUV pipeline on both sides of libass. This prevents
  // FFmpeg from choosing an 8-bit intermediate and creating gradient banding.
  const tenBit = metadata.isHdr || Number(metadata.bitDepth || 0) > 8;
  filters.push(tenBit ? 'format=p010le' : 'format=yuv420p');
  filters.push(subtitleBurn);
  filters.push(tenBit ? 'format=p010le' : 'format=yuv420p');
  return filters.join(',');
}

function previewFrameFilter(metadata, canvas) {
  const filters = [];
  if (canvas.padded) filters.push(`pad=${canvas.width}:${canvas.height}:0:0:black`);
  if (metadata.isHdr) {
    // PNG/Chromium is SDR. Tone-map only the preview so it looks natural on
    // screen; the actual export remains untouched PQ/BT.2020 HDR.
    filters.push('zscale=t=linear:npl=100');
    filters.push('format=gbrpf32le');
    filters.push('zscale=p=bt709');
    filters.push('tonemap=hable:desat=0');
    filters.push('zscale=t=bt709:m=bt709:r=tv');
  }
  filters.push('format=rgb24');
  return filters.join(',');
}

function parseProgressLine(line, duration) {
  const match = String(line).match(/^out_time_(?:ms|us)=(\d+)/m);
  if (!match || !duration) return null;
  return Math.max(0, Math.min(1, Number(match[1]) / 1_000_000 / duration));
}

async function renderPreview(ffmpeg, input) {
  const metadata = input.metadata || await probeMedia(input.ffprobe, input.videoPath);
  const canvas = automaticCanvas(metadata);
  const prepared = await prepareSubtitle(ffmpeg, input.subtitlePath, metadata, input.settings);
  const frame = path.join(prepared.tempDir, 'frame.png');
  const subtitles = path.join(prepared.tempDir, 'subtitles.png');
  const time = previewTimestamp(metadata, input.time);
  const subtitleTime = Math.max(0, time - assNumber(input.settings.delaySeconds, 0));
  const playResY = assNumber(prepared.assText.match(/^PlayResY:\s*([\d.]+)/mi)?.[1], 360);
  const lineCount = activeSubtitleLineCount(prepared.assText, subtitleTime);
  const subtitleLayerFilter = `format=rgba,${subtitleFilter(prepared.prepared, prepared.fontsDir, input.settings.delaySeconds, time, true)}`;
  const frameFilter = previewFrameFilter(metadata, canvas);
  try {
    await Promise.all([run(ffmpeg, [
      '-hide_banner', '-loglevel', 'error', '-y', '-ss', formatNumber(time), '-noautorotate', '-i', input.videoPath,
      '-vf', frameFilter, '-frames:v', '1', '-c:v', 'png', '-pix_fmt', 'rgb24', frame,
    ]), run(ffmpeg, [
      '-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi',
      '-i', `color=c=black@0:s=${canvas.width}x${canvas.height}:r=1,format=rgba,settb=AVTB`,
      '-vf', subtitleLayerFilter, '-frames:v', '1', '-c:v', 'png', '-pix_fmt', 'rgba', subtitles,
    ])]);
    return { frame, subtitles, tempDir: prepared.tempDir, metadata, canvas, fontCount: prepared.fontCount, layout: { playResY, lineCount } };
  } catch (error) {
    await fsp.rm(prepared.tempDir, { recursive: true, force: true });
    throw error;
  }
}

async function exportVideo(ffmpeg, input, handlers = {}) {
  const metadata = input.metadata || await probeMedia(input.ffprobe, input.videoPath);
  const canvas = automaticCanvas(metadata);
  const prepared = await prepareSubtitle(ffmpeg, input.subtitlePath, metadata, input.settings);
  const clipStart = Math.max(0, Math.min(metadata.duration, assNumber(input.clipStart, 0)));
  const clipDuration = input.clipDuration ? Math.max(0.1, Math.min(assNumber(input.clipDuration, 30), metadata.duration - clipStart)) : 0;
  const subtitleBurn = subtitleFilter(prepared.prepared, prepared.fontsDir, input.settings.delaySeconds, null, false, clipStart);
  const filter = exportVideoFilter(metadata, canvas, subtitleBurn);
  const outputExt = path.extname(input.outputPath).toLowerCase();
  const mp4CopyCodecs = new Set(['aac', 'ac3', 'eac3', 'mp3', 'alac']);
  const canCopyAudio = outputExt === '.mkv' || (metadata.audioStreams || []).every(stream => mp4CopyCodecs.has(stream.codec));
  const audioArgs = canCopyAudio ? ['-c:a', 'copy'] : ['-c:a', 'aac', '-b:a', '256k'];
  const encoder = videoEncoderArgs(ffmpeg, input.platform || process.platform, input.settings?.quality, metadata);
  let progressBuffer = '';
  try {
    await run(ffmpeg, [
      '-hide_banner', '-y', ...(clipStart ? ['-ss', formatNumber(clipStart)] : []), '-noautorotate', '-i', input.videoPath,
      '-map', '0:v:0', '-map', '0:a?', '-vf', filter,
      ...encoder.args,
      ...audioArgs, '-map_metadata', '0', '-map_chapters', '0',
      ...(clipDuration ? ['-t', formatNumber(clipDuration)] : []),
      '-fps_mode', 'passthrough', '-progress', 'pipe:1', '-nostats', input.outputPath,
    ], {
      onSpawn: handlers.onSpawn,
      onStdout: chunk => {
        progressBuffer += chunk;
        const lines = progressBuffer.split(/\r?\n/);
        progressBuffer = lines.pop() || '';
        for (const line of lines) {
          const progress = parseProgressLine(line, clipDuration || metadata.duration);
          if (progress !== null) handlers.onProgress?.(progress);
        }
      },
    });
    if (input.ffprobe) {
      const outputMetadata = await probeMedia(input.ffprobe, input.outputPath);
      if (outputMetadata.width !== canvas.width || outputMetadata.height !== canvas.height) {
        throw new Error(`Export safety check failed: expected ${canvas.width}×${canvas.height}, but output is ${outputMetadata.width}×${outputMetadata.height}.`);
      }
      const sourceFps = parseFrameRate(metadata.fps);
      const outputFps = parseFrameRate(outputMetadata.fps);
      if (Math.abs(sourceFps - outputFps) > 0.01) {
        throw new Error(`Export safety check failed: source is ${sourceFps.toFixed(3)} fps, but output is ${outputFps.toFixed(3)} fps.`);
      }
      const sourceAspect = canvas.width / canvas.height;
      const outputAspect = assNumber(outputMetadata.displayAspect, outputMetadata.width / outputMetadata.height);
      if (Math.abs(sourceAspect - outputAspect) > 0.001) {
        throw new Error('Export safety check failed: the output display aspect ratio changed.');
      }
      if (outputMetadata.rotation !== metadata.rotation) {
        throw new Error('Export safety check failed: the output orientation changed.');
      }
      if (metadata.isHdr && !outputMetadata.isHdr) {
        throw new Error('Export safety check failed: HDR color signaling was not preserved.');
      }
      if (metadata.bitDepth > 8 && outputMetadata.bitDepth < 10) {
        throw new Error('Export safety check failed: the 10-bit source was reduced to 8-bit, which would cause banding.');
      }
      if (input.settings?.quality >= 88 && metadata.bitRate > 0 && outputMetadata.bitRate > 0
          && outputMetadata.bitRate < metadata.bitRate * 0.65) {
        throw new Error(`Export safety check failed: the encoder undershot the source video bitrate (${(outputMetadata.bitRate / 1_000_000).toFixed(1)} vs ${(metadata.bitRate / 1_000_000).toFixed(1)} Mbps). Increase quality or use the source-matched fallback.`);
      }
    }
    handlers.onProgress?.(1);
    return {
      output: input.outputPath, fontCount: prepared.fontCount, encoder: encoder.name,
      quality: encoder.quality, hdr: Boolean(encoder.hdr), tenBit: Boolean(encoder.tenBit), dolbyVisionConverted: Boolean(metadata.dolbyVision),
      clipStart, clipDuration, audioCopied: canCopyAudio,
    };
  } finally {
    await fsp.rm(prepared.tempDir, { recursive: true, force: true });
  }
}

module.exports = {
  VIDEO_EXTENSIONS,
  SUBTITLE_EXTENSIONS,
  findTools,
  probeMedia,
  transformAss,
  prepareSubtitle,
  subtitleFilter,
  parseProgressLine,
  parseFrameRate,
  parseRatio,
  previewTimestamp,
  renderPreview,
  exportVideo,
  supportsLibass,
  supportsEncoder,
  videoEncoderArgs,
  colorMetadataArgs,
  sourceAwareBitrateArgs,
  inspectSubtitleTiming,
  activeSubtitleLineCount,
  automaticCanvas,
  exportVideoFilter,
  previewFrameFilter,
};
