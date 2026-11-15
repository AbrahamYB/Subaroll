const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { transformAss, subtitleFilter, parseProgressLine, parseFrameRate, parseRatio, previewTimestamp, videoEncoderArgs, sourceAwareBitrateArgs, activeSubtitleLineCount, automaticCanvas, exportVideoFilter, previewFrameFilter } = require('../lib/burner');

const TEST_FFMPEG = [
  '/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg',
  '/opt/homebrew/bin/ffmpeg',
  '/usr/local/bin/ffmpeg',
].find(candidate => fs.existsSync(candidate)) || 'ffmpeg';

const ASS = `[Script Info]
PlayResX: 1280
PlayResY: 720

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Main,Arial,55,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,3,0,2,25,25,22,1
Style: Sign,Arial,30,&H00112233,&H000000FF,&H00445566,&H00000000,0,0,0,0,100,100,0,0,1,1,0,5,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:03.00,Main,,0,0,0,,Hello
Dialogue: 0,0:00:01.00,0:00:03.00,Sign,,0,0,0,,{\\pos(100,100)}Store
`;

test('styled ASS preserves colors and signs while scaling and moving dialogue', () => {
  const output = transformAss(ASS, { scalePercent: 120, positionPercent: 90, width: 1920, height: 1080 });
  assert.match(output, /Style: Main,Arial,66,&H00FFFFFF,&H000000FF,&H00000000/);
  assert.match(output, /Style: Main,[^\n]*,22,1/);
  assert.match(output, /\{\\an2\\pos\(640,648\)\}Hello/);
  assert.match(output, /Style: Sign,Arial,30,&H00112233/);
  assert.match(output, /\\pos\(100,100\)/);
});

test('encoding quality controls VideoToolbox CQ and stays in the safe UI range', () => {
  assert.equal(videoEncoderArgs('/usr/bin/true', 'darwin', 72).quality, 72);
  assert.equal(videoEncoderArgs('/usr/bin/true', 'darwin', 999).quality, 100);
  assert.equal(videoEncoderArgs('/usr/bin/true', 'darwin', 1).quality, 60);
});

test('encoding quality maps to x264 CRF on Windows', () => {
  const high = videoEncoderArgs('ffmpeg', 'win32', 90);
  const compact = videoEncoderArgs('ffmpeg', 'win32', 70);
  assert.equal(high.args[high.args.indexOf('-crf') + 1], '14');
  assert.equal(compact.args[compact.args.indexOf('-crf') + 1], '20');
});

test('known source bitrate caps VideoToolbox output size', () => {
  const args = sourceAwareBitrateArgs({ bitRate: 18_333_751 }, 90);
  const target = Number(args[args.indexOf('-b:v') + 1]);
  assert.equal(target, 18_333_751);
  assert.equal(Number(args[args.indexOf('-bufsize') + 1]), target * 2);
});

test('plain subtitle canvas follows cinematic display aspect without stretching', () => {
  const output = transformAss(ASS, { plain: true, width: 1920, height: 804, displayAspect: 2.388, scalePercent: 100, positionPercent: 100 });
  assert.match(output, /PlayResX: 860/);
  assert.match(output, /PlayResY: 360/);
});

test('absolute subtitle position reaches visible top and bottom edges', () => {
  const top = transformAss(ASS, { plain: true, width: 1920, height: 1080, positionPercent: 0, scalePercent: 100 });
  const bottom = transformAss(ASS, { plain: true, width: 1920, height: 1080, positionPercent: 100, scalePercent: 100 });
  assert.match(top, /\{\\an2\\pos\(320,34\)\}Hello/);
  assert.match(bottom, /\{\\an2\\pos\(320,356\)\}Hello/);
  assert.match(bottom, /\{\\pos\(100,100\)\}Store/);
});

test('display and sample aspect ratios parse safely', () => {
  assert.equal(parseRatio('16:9'), 16 / 9);
  assert.equal(parseRatio('4/3'), 4 / 3);
  assert.equal(parseRatio('N/A', 1), 1);
});

test('plain subtitles receive the Crunchyroll dialogue recipe', () => {
  const output = transformAss(ASS, { plain: true, scalePercent: 100, positionPercent: 100, width: 1920, height: 1080 });
  assert.match(output, /PlayResX: 640/);
  assert.match(output, /PlayResY: 360/);
  assert.match(output, /ScaledBorderAndShadow: yes/);
  assert.match(output, /Style: Main,Trebuchet MS,24,&H00FFFFFF,[^\n]*&H00000000/);
  assert.match(output, /,1,2,1,2,10,10,18,1/);
});

test('delay filter shifts only subtitle evaluation and restores video timestamps', () => {
  const filter = subtitleFilter('/tmp/sub.ass', '/tmp/fonts', 1.5);
  assert.match(filter, /^setpts=PTS-1\.5\/TB,subtitles=/);
  assert.match(filter, /,setpts=PTS\+1\.5\/TB$/);
});

test('interactive preview requests alpha-aware subtitle rendering', () => {
  const filter = subtitleFilter('/tmp/sub.ass', '/tmp/fonts', 0, 3.5, true);
  assert.match(filter, /:alpha=1/);
  assert.match(filter, /setpts=PTS\+3\.5\/TB/);
  const burnerSource = fs.readFileSync(path.join(__dirname, '..', 'lib', 'burner.js'), 'utf8');
  assert.match(burnerSource, /format=rgba,settb=AVTB/);
});

test('test clips evaluate subtitles on the original video timeline', () => {
  const filter = subtitleFilter('/tmp/sub.ass', '/tmp/fonts', 0, null, false, 125.5);
  assert.match(filter, /^setpts=PTS\+125\.5\/TB,subtitles=/);
  assert.match(filter, /,setpts=PTS-125\.5\/TB$/);
});

test('position and size sliders never schedule a rendered preview', () => {
  const renderer = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'renderer.js'), 'utf8');
  assert.doesNotMatch(renderer, /position\.addEventListener\('change'/);
  assert.doesNotMatch(renderer, /scale\.addEventListener\('change'/);
  const liveAdjustment = renderer.match(/function beginLiveSubtitleAdjustment\(\) \{([\s\S]*?)\n\}/)?.[1] || '';
  assert.doesNotMatch(liveAdjustment, /previewRequest/);
  assert.match(liveAdjustment, /applyLiveSubtitleTransform\(\)/);
});

test('subtitle output filename records every user-selected setting', () => {
  const renderer = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'renderer.js'), 'utf8');
  assert.match(renderer, /`pos\$\{position\.toFixed\(2\)\} sub\$\{delayToken\} size\$\{size\.toFixed\(3\)\} cq\$\{Math\.round/);
  assert.match(renderer, /elements\.delay\.addEventListener\('input',[\s\S]*updateAutomaticOutput\(\)/);
  assert.doesNotMatch(renderer, /elements\.outputPicker/);
});

test('cinematic preview automatically uses a 16:9 canvas', () => {
  const renderer = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'renderer.js'), 'utf8');
  assert.match(renderer, /previewStage\.style\.aspectRatio = `\$\{canvas\.width\} \/ \$\{canvas\.height\}`/);
  assert.doesNotMatch(renderer, /letterbox/);
});

test('cinematic sources get automatic bottom-only padding while 16:9 stays untouched', () => {
  assert.deepEqual(automaticCanvas({ width: 3840, height: 1600 }), { width: 3840, height: 2160, bottomPadding: 560, padded: true });
  assert.deepEqual(automaticCanvas({ width: 1920, height: 1080 }), { width: 1920, height: 1080, bottomPadding: 0, padded: false });
  assert.deepEqual(automaticCanvas({ width: 1080, height: 1920 }), { width: 1080, height: 1920, bottomPadding: 0, padded: false });
});

test('live positioning uses the active cue line count', () => {
  assert.equal(activeSubtitleLineCount(ASS, 2), 1);
  const twoLines = ASS.replace('Hello', 'Hello\\NSecond line');
  assert.equal(activeSubtitleLineCount(twoLines, 2), 2);
  assert.equal(activeSubtitleLineCount(twoLines, 10), 1);
});

test('progress parser reads FFmpeg microseconds', () => {
  assert.equal(parseProgressLine('out_time_us=5000000', 10), 0.5);
  assert.equal(parseProgressLine('progress=continue', 10), null);
});

test('frame-rate parser supports FFprobe fractions', () => {
  assert.equal(parseFrameRate('30000/1001').toFixed(3), '29.970');
  assert.equal(parseFrameRate('24/1'), 24);
  assert.equal(parseFrameRate(''), 25);
});

test('preview timestamp stays inside the final video frame', () => {
  assert.equal(previewTimestamp({ duration: 30, fps: '30/1' }, 30), 29.9);
  assert.equal(previewTimestamp({ duration: 30, fps: '60/1' }, 30), 29.9);
  assert.equal(previewTimestamp({ duration: 30, fps: '30/1' }, 10), 10);
});

test('non-macOS builds retain the portable software encoder', () => {
  const encoder = videoEncoderArgs('ffmpeg', 'win32');
  assert.equal(encoder.name, 'libx264');
  assert.deepEqual(encoder.args.slice(0, 4), ['-c:v', 'libx264', '-preset', 'medium']);
});

test('macOS hardware encoder uses the high-quality VideoToolbox recipe when available', (t) => {
  const encoder = videoEncoderArgs(TEST_FFMPEG, 'darwin');
  if (encoder.name !== 'h264_videotoolbox') return t.skip('VideoToolbox encoder unavailable');
  assert.equal(encoder.args[encoder.args.indexOf('-q:v') + 1], '90');
  assert.equal(encoder.args[encoder.args.indexOf('-prio_speed') + 1], '0');
  assert.equal(encoder.args[encoder.args.indexOf('-allow_sw') + 1], '0');
  assert.equal(encoder.args[encoder.args.indexOf('-spatial_aq') + 1], '1');
});

test('known source bitrate cannot force VideoToolbox back into undershooting bitrate mode', (t) => {
  const encoder = videoEncoderArgs(TEST_FFMPEG, 'darwin', 90, { bitRate: 18_333_751 });
  if (encoder.name !== 'h264_videotoolbox') return t.skip('VideoToolbox encoder unavailable');
  assert.equal(encoder.args[encoder.args.indexOf('-q:v') + 1], '90');
  assert.equal(encoder.args.includes('-b:v'), false);
  assert.equal(encoder.args.includes('-constant_bit_rate'), false);
});

test('exports hold a system sleep assertion until encoding finishes', () => {
  const rustSource = fs.readFileSync(path.join(__dirname, '..', 'src-tauri', 'src', 'media.rs'), 'utf8');
  assert.match(rustSource, /Command::new\("caffeinate"\)/);
  assert.match(rustSource, /\.args\(\["-i", "-w"/);
});

test('Windows selects usable NVIDIA, AMD, or Intel hardware before software fallback', () => {
  const rustSource = fs.readFileSync(path.join(__dirname, '..', 'src-tauri', 'src', 'media.rs'), 'utf8');
  for (const encoder of ['h264_nvenc', 'hevc_nvenc', 'h264_amf', 'hevc_amf', 'h264_qsv', 'hevc_qsv']) {
    assert.match(rustSource, new RegExp(encoder));
  }
  assert.match(rustSource, /encoder_usable/);
  assert.match(rustSource, /libx265/);
  assert.match(rustSource, /libx264/);
});

test('Windows UI exposes detected encoder and CTT preset controls only on Windows', () => {
  const html = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'index.html'), 'utf8');
  const renderer = fs.readFileSync(path.join(__dirname, '..', 'renderer', 'renderer.js'), 'utf8');
  for (const value of ['auto', 'nvenc', 'amf', 'qsv', 'software']) {
    assert.match(html, new RegExp(`option value="${value}"`));
  }
  for (const preset of ['quality', 'balanced', 'fast']) {
    assert.match(html, new RegExp(`option value="${preset}"`));
  }
  assert.match(html, /CTT Quality/);
  assert.match(renderer, /state\.platform === 'windows'/);
  assert.match(renderer, /option\.disabled = !available\.has/);
});

test('HDR uses 10-bit HEVC VideoToolbox with preserved PQ color signaling', (t) => {
  const encoder = videoEncoderArgs(TEST_FFMPEG, 'darwin', 90, {
    isHdr: true, colorRange: 'tv', colorSpace: 'bt2020nc',
    colorTransfer: 'smpte2084', colorPrimaries: 'bt2020',
  });
  if (encoder.name !== 'hevc_videotoolbox') return t.skip('HEVC VideoToolbox encoder unavailable');
  assert.equal(encoder.args[encoder.args.indexOf('-profile:v') + 1], 'main10');
  assert.equal(encoder.args[encoder.args.indexOf('-pix_fmt') + 1], 'p010le');
  assert.equal(encoder.args[encoder.args.indexOf('-color_trc') + 1], 'smpte2084');
  assert.equal(encoder.args[encoder.args.indexOf('-color_primaries') + 1], 'bt2020');
});

test('HDR export keeps a 10-bit pipeline across padding and libass', () => {
  const filter = exportVideoFilter(
    { isHdr: true },
    { width: 3840, height: 2160, padded: true },
    "subtitles=filename='prepared.ass'",
  );
  assert.equal(filter, "pad=3840:2160:0:0:black,format=p010le,subtitles=filename='prepared.ass',format=p010le");
});

test('10-bit SDR anime uses HEVC Main10 and never an 8-bit filter path', (t) => {
  const metadata = {
    isHdr: false, bitDepth: 10, colorRange: 'tv', colorSpace: 'bt709',
    colorTransfer: 'bt709', colorPrimaries: 'bt709',
  };
  const encoder = videoEncoderArgs(TEST_FFMPEG, 'darwin', 90, metadata);
  if (encoder.name !== 'hevc_videotoolbox') return t.skip('HEVC VideoToolbox encoder unavailable');
  assert.equal(encoder.args[encoder.args.indexOf('-profile:v') + 1], 'main10');
  assert.equal(encoder.args[encoder.args.indexOf('-pix_fmt') + 1], 'p010le');
  assert.equal(encoder.hdr, false);
  assert.equal(encoder.tenBit, true);
  assert.equal(
    exportVideoFilter(metadata, { width: 1920, height: 1080, padded: false }, "subtitles=filename='anime.ass'"),
    "format=p010le,subtitles=filename='anime.ass',format=p010le",
  );
});

test('8-bit macOS sources use HEVC Main and direct CQ instead of H.264', (t) => {
  const encoder = videoEncoderArgs(TEST_FFMPEG, 'darwin', 90, {
    isHdr: false, bitDepth: 8, colorRange: 'tv', colorSpace: 'bt709',
    colorTransfer: 'bt709', colorPrimaries: 'bt709',
  });
  if (encoder.name !== 'hevc_videotoolbox') return t.skip('HEVC VideoToolbox encoder unavailable');
  assert.equal(encoder.args[encoder.args.indexOf('-profile:v') + 1], 'main');
  assert.equal(encoder.args[encoder.args.indexOf('-pix_fmt') + 1], 'yuv420p');
  assert.equal(encoder.args[encoder.args.indexOf('-q:v') + 1], '90');
  assert.equal(encoder.tenBit, false);
});

test('HDR preview is tone-mapped for an SDR PNG without changing export color', () => {
  const filter = previewFrameFilter({ isHdr: true }, { width: 3840, height: 2160, padded: true });
  assert.match(filter, /^pad=3840:2160:0:0:black,zscale=t=linear/);
  assert.match(filter, /tonemap=hable/);
  assert.match(filter, /zscale=t=bt709:m=bt709:r=tv,format=rgb24$/);
});
