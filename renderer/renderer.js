const api = window.subaroll;
document.documentElement.classList.toggle('platform-windows', /Windows/.test(navigator.userAgent));
const state = {
  videoPath: '', subtitlePath: '', outputPath: '', metadata: null,
  mode: 'subtitles', exporting: false, previewTimer: null, previewRequest: 0, previewBusy: false, previewPending: null, previewDirty: false, previewFrameUrl: null, videoEncoder: '', platform: '', availableVideoEncoders: [], availableAv1Encoders: [], subtitleTiming: null, subtitleWarning: '',
  availableUpdate: null, updateBusy: false, autoVideoEncoder10bit: '',
  appleBurnAvailable: false, toolsReady: false,
};

const $ = id => document.getElementById(id);
const elements = {
  videoPicker: $('videoPicker'), subtitlePicker: $('subtitlePicker'),
  videoName: $('videoName'), subtitleName: $('subtitleName'),
  delay: $('delay'), delayValue: $('delayValue'), position: $('position'), positionValue: $('positionValue'),
  scale: $('scale'), scaleValue: $('scaleValue'), seek: $('seek'), seekCurrent: $('seekCurrent'), seekDuration: $('seekDuration'),
  quality: $('quality'), qualityValue: $('qualityValue'), qualityHint: $('qualityHint'),
  windowsEncoderCard: $('windowsEncoderCard'), windowsEncoder: $('windowsEncoder'),
  encoderAvailability: $('encoderAvailability'), encoderHint: $('encoderHint'),
  previewStage: $('previewStage'), previewFrame: $('previewFrame'), previewSpinner: $('previewSpinner'),
  exportButton: $('exportButton'), testButton: $('testButton'), cancelButton: $('cancelButton'), progressWrap: $('progressWrap'),
  progressValue: $('progressValue'), progressBar: $('progressBar'), status: $('status'),
  subtitleMode: $('subtitleMode'), compressMode: $('compressMode'), compressionOptions: $('compressionOptions'),
  targetSize: $('targetSize'), estRes: $('estRes'), estFps: $('estFps'), estRate: $('estRate'), estNote: $('estNote'), estimate: $('estimate'), previewNote: document.querySelector('.preview-note'), subtitleOnly: [...document.querySelectorAll('.subtitle-only')],
  targetDown: $('targetDown'), targetUp: $('targetUp'),
  compressionCodec: $('compressionCodec'), compressionCodecHint: $('compressionCodecHint'),
  modeEyebrow: $('modeEyebrow'), modeTitle: $('modeTitle'),
  updateButton: $('updateButton'),
};

// Custom slider: pill track, dotted fill, capsule thumb. The real <input type=range> stays on top
// (invisible) so the rest of the app still reads/writes its value. While dragging, values near a
// landmark softly lock. The highlight follows the current value for every input method.
function enhanceSlider(box) {
  const input = box.querySelector('input[type=range]');
  const min = Number(input.min), max = Number(input.max), span = max - min;
  const pct = v => (v - min) / span * 100;
  const at = p => `calc(${p}% + ${(11 - 0.22 * p).toFixed(2)}px)`;
  const marks = (box.dataset.marks || '').split(';').filter(Boolean).map(m => m.split(':'));
  const snap = [...marks.map(m => Number(m[0])), ...(box.dataset.snap || '').split(',').filter(Boolean).map(Number)];
  const ends = document.createElement('div');
  ends.className = 'ends';
  ends.innerHTML = `<span>${box.dataset.lo || ''}</span><span>${box.dataset.hi || ''}</span>`;
  const ns = document.createElement('div');
  ns.className = 'ns';
  ns.innerHTML = '<div class="tr"><div class="fl"></div></div><div class="th"></div>';
  const tr = ns.querySelector('.tr'), fill = ns.querySelector('.fl'), thumb = ns.querySelector('.th');
  const ticks = marks.map(([v]) => { const tick = document.createElement('div'); tick.className = 'tk'; tick.style.left = at(pct(Number(v))); tr.appendChild(tick); return tick; });
  const caption = document.createElement('div');
  caption.className = 'mk';
  marks.forEach(([v, text]) => { const label = document.createElement('span'); label.textContent = text; label.style.left = at(pct(Number(v))); caption.appendChild(label); });
  input.before(ends, ns);
  ns.appendChild(input);
  box.appendChild(caption);

  // Detent zone is a fixed pixel width around each landmark, independent of window size.
  const ZONE_PX = 9;
  let dragging = false;
  const draw = () => {
    const value = Number(input.value);
    const p = pct(value);
    fill.style.width = at(p);
    thumb.style.left = `calc(${p}% - ${(0.22 * p).toFixed(2)}px)`;
    const hit = snap.find(point => Math.abs(value - point) < 1e-6);
    ns.classList.toggle('lock', hit !== undefined);
    ticks.forEach((tick, i) => tick.classList.toggle('hit', Number(marks[i][0]) === hit));
  };
  // Magnetic detent: while dragging, the pointer range [point-Z, point+Z] maps onto the point itself and
  // everything outside maps linearly back to the ends. The mapping is continuous, so the thumb never
  // jumps, you can still land right next to a point, and there is no direction state to get stuck in.
  const knots = () => {
    const z = Math.min(ZONE_PX * span / Math.max(40, ns.clientWidth - 22), span * 0.08);
    const xs = [min], ys = [min];
    for (const p of [...snap].sort((x, y) => x - y)) {
      if (p - z <= min || p + z >= max) continue; // never let a detent swallow an end of the slider
      xs.push(p - z, p + z); ys.push(p, p);
    }
    xs.push(max); ys.push(max);
    return { xs, ys, z };
  };
  const remap = raw => {
    const { xs, ys } = knots();
    for (let i = 1; i < xs.length; i++) {
      if (raw <= xs[i]) {
        const width = xs[i] - xs[i - 1];
        return width > 0 ? ys[i - 1] + (ys[i] - ys[i - 1]) * (raw - xs[i - 1]) / width : ys[i];
      }
    }
    return max;
  };
  input.addEventListener('input', () => {
    if (!dragging || !snap.length) { draw(); return; }
    const value = remap(Number(input.value));
    input.value = String(value);
    draw();
  });
  input.addEventListener('pointerdown', () => { dragging = true; ns.classList.add('drag'); });
  input.addEventListener('change', draw);
  const release = () => {
    if (!dragging) return;
    dragging = false;
    ns.classList.remove('drag');
    draw();
  };
  window.addEventListener('pointerup', release);
  window.addEventListener('pointercancel', release);
  input.refreshSlider = draw;
  draw();
  return caption;
}
// Landmark labels that would overlap a neighbour (narrow window) are hidden rather than squashed.
function declutterCaptions() {
  for (const caption of sliderCaptions) {
    let right = -Infinity;
    for (const label of caption.children) {
      label.style.visibility = '';
      const box = label.getBoundingClientRect();
      if (box.left < right + 6 || box.left < caption.getBoundingClientRect().left - 4 || box.right > caption.getBoundingClientRect().right + 4) label.style.visibility = 'hidden';
      else right = box.right;
    }
  }
}

function basename(filePath) { return filePath.split(/[\\/]/).pop() || filePath; }
function extless(filePath) { return basename(filePath).replace(/\.[^.]+$/, ''); }
function dirname(filePath) { const index = Math.max(filePath.lastIndexOf('/'), filePath.lastIndexOf('\\')); return index >= 0 ? filePath.slice(0, index + 1) : ''; }
function formatTime(seconds) {
  const total = Math.max(0, Math.round(Number(seconds) || 0));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  return hours ? `${hours}:${String(minutes).padStart(2, '0')}:${String(secs).padStart(2, '0')}` : `${String(minutes).padStart(2, '0')}:${String(secs).padStart(2, '0')}`;
}
function publicReleaseVersion(version) {
  const match = String(version).match(/^(\d+)\.(\d+)(?:\.|$)/);
  return match ? `${match[1]}.${match[2]}` : String(version);
}
function previewCanvas(metadata) {
  const width = Number(metadata?.width) || 16;
  const height = Number(metadata?.height) || 9;
  if (width / height <= (16 / 9) + 0.01) return { width, height, padded: false };
  return { width, height: Math.ceil((width * 9 / 16) / 2) * 2, padded: true };
}
function settings() {
  return {
    delaySeconds: Number(elements.delay.value), positionPercent: Number(elements.position.value),
    scalePercent: Number(elements.scale.value), quality: Number(elements.quality.value),
    windowsEncoder: elements.windowsEncoder.value, windowsPreset: 'quality', // the Quality slider is the only quality control; encoders always use their best preset
  };
}
const SETTINGS_KEY = 'subaroll.subtitleSettings';
const LEGACY_SETTINGS_KEY = 'subtitleBurner.subtitleSettings';
const COMPRESSION_CODEC_KEY = 'subaroll.compressionCodec';
const LEGACY_COMPRESSION_CODEC_KEY = 'subtitleBurner.compressionCodec';
const COMPRESSION_TARGET_KEY = 'subaroll.compressionTargetMB';
function saveSubtitleSettings() {
  localStorage.setItem(SETTINGS_KEY, JSON.stringify({ ...settings(), settingsVersion: 4 }));
}
function restoreSubtitleSettings() {
  try {
    const currentSettings = localStorage.getItem(SETTINGS_KEY);
    const existing = currentSettings || localStorage.getItem(LEGACY_SETTINGS_KEY);
    const legacyQualityValue = localStorage.getItem('subtitleBurner.encodingQuality');
    const legacyQuality = legacyQualityValue === null ? NaN : Number(legacyQualityValue);
    const saved = existing ? JSON.parse(existing) : {
      delaySeconds: 0,
      positionPercent: 95,
      scalePercent: 115,
      quality: Number.isFinite(legacyQuality) ? legacyQuality : 90,
    };
    if (Number.isFinite(saved.delaySeconds)) elements.delay.value = String(Math.max(-5, Math.min(5, saved.delaySeconds)));
    if (Number.isFinite(saved.positionPercent)) {
      const position = saved.settingsVersion === 2 || saved.settingsVersion >= 4
        ? saved.positionPercent
        : 95 + (saved.positionPercent - 100);
      elements.position.value = String(Math.max(0, Math.min(100, position)));
    }
    if (Number.isFinite(saved.scalePercent)) elements.scale.value = String(Math.max(50, Math.min(200, saved.scalePercent)));
    if (Number.isFinite(saved.quality)) elements.quality.value = String(Math.max(60, Math.min(100, saved.quality)));
    if (['auto', 'nvenc', 'amf', 'qsv', 'videotoolbox', 'software'].includes(saved.windowsEncoder)) elements.windowsEncoder.value = saved.windowsEncoder;
    if (saved.settingsVersion !== 4) saveSubtitleSettings();
  } catch (_) {}
  const position = Number(elements.position.value);
  elements.delayValue.textContent = `${Number(elements.delay.value).toFixed(2)} s`;
  elements.positionValue.textContent = position === 0 ? 'Top' : position === 50 ? 'Middle' : position === 100 ? 'Bottom' : `${position}% from top`;
  elements.scaleValue.textContent = `${elements.scale.value}%`;
}
function qualityDescription(value) {
  if (value >= 96) return ['Maximum', 'Near-lossless hardware quality with a source-aware bitrate ceiling.'];
  if (value >= 88) return ['Visually lossless', 'Hardware constant-quality mode with a ceiling derived from the source bitrate.'];
  if (value >= 76) return ['High', 'Very good quality with a smaller output file.'];
  return ['Compact', 'Smallest files; fine detail may be less crisp.'];
}
function refreshQualityDisplay() {
  const value = Number(elements.quality.value);
  const [label, hint] = qualityDescription(value);
  elements.qualityValue.textContent = `CQ ${value}`;
  elements.qualityHint.textContent = `${label}. ${hint}`;
}
function autoEncoderLabel() {
  const tenBit = state.metadata?.isHdr || state.metadata?.bitDepth > 8;
  const encoder = tenBit ? state.autoVideoEncoder10bit : state.videoEncoder;
  const labels = {
    h264_nvenc: 'NVIDIA NVENC (H.264)', hevc_nvenc: 'NVIDIA NVENC (HEVC)',
    h264_amf: 'AMD AMF (H.264)', hevc_amf: 'AMD AMF (HEVC)',
    h264_qsv: 'Intel Quick Sync (H.264)', hevc_qsv: 'Intel Quick Sync (HEVC)',
    hevc_videotoolbox: 'Apple VideoToolbox (HEVC)',
    libx264: 'CPU (H.264)', libx265: 'CPU (HEVC)',
  };
  return labels[encoder] || '';
}
function encoderPresetDescription() {
  const autoLabel = autoEncoderLabel();
  elements.windowsEncoder.options[0].textContent = autoLabel ? `Auto · ${autoLabel}` : 'Auto';
  elements.encoderHint.textContent = `${elements.windowsEncoder.selectedOptions[0].textContent}. Codec, 10-bit, HDR, resolution, and color metadata remain source-aware.`;
}
function syncEncoderChips() {
  const select = elements.windowsEncoder;
  document.querySelectorAll('#encoderChips .chip').forEach(chip => {
    const option = [...select.options].find(item => item.value === chip.dataset.value);
    chip.disabled = select.disabled || Boolean(option?.disabled);
    chip.classList.toggle('on', select.value === chip.dataset.value);
    chip.setAttribute('aria-checked', String(select.value === chip.dataset.value));
  });
}
document.querySelectorAll('#encoderChips .chip').forEach(chip => chip.addEventListener('click', () => {
  if (chip.disabled) return;
  elements.windowsEncoder.value = chip.dataset.value;
  elements.windowsEncoder.dispatchEvent(new Event('change', { bubbles: true }));
}));
function syncCodecChips() {
  const select = elements.compressionCodec;
  document.querySelectorAll('#codecChips .chip').forEach(chip => {
    const option = [...select.options].find(item => item.value === chip.dataset.value);
    const on = select.value === chip.dataset.value;
    chip.disabled = Boolean(option?.disabled);
    chip.classList.toggle('on', on);
    chip.setAttribute('aria-checked', String(on));
  });
}
document.querySelectorAll('#codecChips .chip').forEach(chip => chip.addEventListener('click', () => {
  elements.compressionCodec.value = chip.dataset.value;
  elements.compressionCodec.dispatchEvent(new Event('change', { bubbles: true }));
}));
function setStatus(message, type = '') {
  elements.status.onclick = null;
  elements.status.textContent = message;
  elements.status.title = message;
  elements.status.className = `status ${type}`;
}
async function checkForUpdates(manual = false) {
  if (state.updateBusy) return;
  state.updateBusy = true;
  elements.updateButton.disabled = true;
  elements.updateButton.textContent = 'Checking…';
  try {
    const { installedVersion, update } = await api.checkForUpdate();
    state.availableUpdate = update;
    if (update) {
      elements.updateButton.textContent = `Install v${publicReleaseVersion(update.version)}`;
      elements.updateButton.title = [`Installed build ${installedVersion}. Available build ${update.version}.`, update.notes].filter(Boolean).join('\n\n');
      if (manual) setStatus(`Subaroll ${publicReleaseVersion(update.version)} is ready to install.`);
    } else {
      elements.updateButton.textContent = 'Check for updates';
      elements.updateButton.title = `Installed build ${installedVersion}. Check GitHub for a newer build.`;
      if (manual) setStatus('Subaroll is up to date.', 'success');
    }
  } catch (error) {
    elements.updateButton.textContent = 'Check for updates';
    if (manual) setStatus(`Update check failed: ${error}`, 'error');
    else console.warn('Subaroll update check failed:', error);
  } finally {
    state.updateBusy = false;
    elements.updateButton.disabled = false;
  }
}
async function installAvailableUpdate() {
  if (!state.availableUpdate || state.updateBusy) return;
  state.updateBusy = true;
  elements.updateButton.disabled = true;
  elements.updateButton.textContent = 'Installing…';
  setStatus(`Downloading and installing Subaroll ${publicReleaseVersion(state.availableUpdate.version)}…`);
  try {
    await api.installUpdate();
    setStatus('Update installed. Restarting Subaroll…');
  } catch (error) {
    const noLongerAvailable = String(error).includes('No update is available.');
    if (noLongerAvailable) state.availableUpdate = null;
    setStatus(noLongerAvailable ? 'That update is no longer available. Check again.' : `Update failed: ${error}`, noLongerAvailable ? '' : 'error');
    elements.updateButton.textContent = state.availableUpdate ? `Install v${publicReleaseVersion(state.availableUpdate.version)}` : 'Check for updates';
    elements.updateButton.disabled = false;
    state.updateBusy = false;
  }
}
// Fixed-size window: nothing to fit, only hide landmark labels that would collide.
const stageCol = $('stageCol'), dock = $('dock');
function fitWindowToContent() { declutterCaptions(); }

function refreshReady() {
  const hasRequired = state.videoPath && state.outputPath && state.metadata &&
    (state.mode === 'compress' || state.subtitlePath);
  elements.exportButton.disabled = state.exporting || !hasRequired;
  elements.testButton.disabled = state.exporting || !hasRequired;
}
function subtitleFilenameToken() {
  const position = Math.max(0, Math.min(100, Number(elements.position.value))) / 100;
  const delay = Number(elements.delay.value);
  const delayToken = `${delay < 0 ? 'm' : ''}${String(Math.round(Math.abs(delay) * 100)).padStart(3, '0')}`;
  const size = Math.max(0, Number(elements.scale.value)) / 1000;
  return `pos${position.toFixed(2)} sub${delayToken} size${size.toFixed(3)} cq${Math.round(Number(elements.quality.value))}`;
}
function suggestOutput() {
  if (!state.videoPath) return '';
  const suffix = state.mode === 'compress'
    ? `${Math.round(Number(elements.targetSize.value) || 25)}MB compressed`
    : subtitleFilenameToken();
  return `${dirname(state.videoPath)}${extless(state.videoPath)} — ${suffix}.mp4`;
}
function updateAutomaticOutput() { state.outputPath = suggestOutput(); }

function setMode(mode) {
  if (state.exporting) return;
  const previousMode = state.mode;
  state.mode = mode === 'compress' ? 'compress' : 'subtitles';
  const compressing = state.mode === 'compress';
  elements.subtitleMode.classList.toggle('active', !compressing);
  elements.compressMode.classList.toggle('active', compressing);
  dock.classList.toggle('compress', compressing);
  // inert (not hidden) so both layouts occupy the same dock and cross-fade.
  document.querySelectorAll('.compress-only').forEach(element => element.toggleAttribute('inert', !compressing));
  elements.subtitleOnly.forEach(element => element.toggleAttribute('inert', compressing));
  elements.previewNote.textContent = compressing
    ? 'Compression keeps the original aspect ratio and chooses bitrate and resolution for the target size.'
    : 'Libass renders the preview with the current subtitle delay, position, and size.';
  elements.exportButton.textContent = compressing ? 'Compress video' : 'Burn subtitles';
  elements.modeEyebrow.textContent = compressing ? 'TARGET-SIZE COMPRESSION' : 'LIBASS BURN-IN';
  elements.modeTitle.textContent = compressing ? 'Compress video' : 'Burn subtitles';
  updateAutomaticOutput();
  refreshEncoderAvailability();
  updateEstimate();
  setStatus('');
  if (state.mode !== previousMode) {
    schedulePreview();
  }
  refreshReady();
  fitWindowToContent();
}

function refreshEncoderAvailability() {
  const windows = state.platform === 'windows';
  const mac = state.platform === 'macos';
  const usable = state.toolsReady && (windows || mac);
  elements.windowsEncoder.disabled = !usable;
  elements.windowsEncoderCard.classList.toggle('is-unavailable', !usable);
  const available = new Set(state.availableVideoEncoders);
  if (mac && state.mode === 'subtitles') {
    available.delete('videotoolbox');
    if (state.appleBurnAvailable) available.add('videotoolbox');
  }
  for (const option of elements.windowsEncoder.options) {
    option.disabled = !['auto', 'software'].includes(option.value) && !available.has(option.value);
  }
  if (usable && elements.windowsEncoder.selectedOptions[0]?.disabled) elements.windowsEncoder.value = 'auto';
  syncEncoderChips();
}

async function checkTools() {
  const result = await api.toolStatus();
  state.platform = result.platform || '';
  const windows = state.platform === 'windows';
  const mac = state.platform === 'macos';
  document.documentElement.classList.toggle('platform-windows', windows);
  document.documentElement.classList.toggle('platform-macos', mac);
  state.toolsReady = result.ok;
  elements.encoderAvailability.textContent = windows || mac ? 'Detecting…' : 'Unavailable';
  if (result.ok) {
    state.videoEncoder = result.videoEncoder;
    state.autoVideoEncoder10bit = result.autoVideoEncoder10bit;
    state.availableVideoEncoders = result.availableVideoEncoders || [];
    state.appleBurnAvailable = Boolean(result.appleBurnAvailable);
    state.availableAv1Encoders = result.availableAv1Encoders || [];
    const av1Option = elements.compressionCodec.querySelector('option[value="av1"]');
    av1Option.disabled = state.availableAv1Encoders.length === 0;
    const savedCodec = localStorage.getItem(COMPRESSION_CODEC_KEY) || localStorage.getItem(LEGACY_COMPRESSION_CODEC_KEY);
    if (['auto', 'av1', 'h264'].includes(savedCodec)) elements.compressionCodec.value = savedCodec;
    if (savedCodec && !localStorage.getItem(COMPRESSION_CODEC_KEY)) localStorage.setItem(COMPRESSION_CODEC_KEY, savedCodec);
    if (elements.compressionCodec.selectedOptions[0]?.disabled) elements.compressionCodec.value = 'auto';
    const av1Labels = { nvenc: 'NVIDIA NVENC', qsv: 'Intel Quick Sync', amf: 'AMD AMF' };
    elements.compressionCodecHint.textContent = state.availableAv1Encoders.length
      ? `AV1 ready on ${state.availableAv1Encoders.map(value => av1Labels[value]).join(' + ')}.`
      : 'No hardware AV1 found. Auto uses H.264.';
    document.querySelector('#codecChips [data-value=av1]').title = elements.compressionCodecHint.textContent;
    if (windows || mac) {
      const labels = { nvenc: 'NVIDIA', amf: 'AMD', qsv: 'Intel', videotoolbox: 'Apple' };
      elements.encoderAvailability.textContent = state.availableVideoEncoders.length
        ? state.availableVideoEncoders.map(value => labels[value]).join(' + ') + ' detected'
        : 'CPU only';
      encoderPresetDescription();
    }
  } else {
    elements.encoderAvailability.textContent = 'Unavailable';
    setStatus('FFmpeg with libass is required.', 'error');
  }
  refreshEncoderAvailability();
  updateEstimate();
}

async function setVideo(videoPath) {
  if (!videoPath) return;
  clearTimeout(state.previewTimer);
  state.previewRequest += 1;
  state.previewPending = null;
  elements.previewStage.classList.remove('has-image');
  releasePreviewFrame();
  state.previewDirty = false;
  elements.previewSpinner.hidden = true;
  elements.seek.disabled = true;
  state.videoPath = videoPath;
  state.metadata = null;
  refreshReady();
  elements.estFps.textContent = '—';
  updateEstimate();
  elements.videoName.textContent = basename(videoPath);
  setStatus('Reading video details…');
  try {
    const metadata = await api.probe(videoPath);
    if (state.videoPath !== videoPath) return;
    state.metadata = metadata;
    const [frames, seconds = 1] = String(metadata.fps ?? '').split('/').map(Number);
    const sourceFps = frames / seconds;
    if (Number.isFinite(sourceFps) && sourceFps > 0) {
      elements.estFps.textContent = `${Number(sourceFps.toFixed(3))} fps`;
    }
    const canvas = previewCanvas(state.metadata);
    elements.seek.max = String(state.metadata.duration || 0);
    elements.seek.value = String(Math.min(30, Math.max(0, (state.metadata.duration || 0) * .08)));
    elements.seek.disabled = !state.metadata.duration;
    elements.seekDuration.textContent = formatTime(state.metadata.duration);
    elements.seekCurrent.textContent = formatTime(elements.seek.value);
    updateAutomaticOutput();
    updateEstimate();
    updateSubtitleWarning();
    if (state.metadata.isHdr) {
      setStatus(`${state.metadata.width}×${state.metadata.height}${canvas.padded ? ` → ${canvas.width}×${canvas.height} with automatic lower subtitle area` : ''} • ${formatTime(state.metadata.duration)} • ${state.metadata.dolbyVision ? 'Dolby Vision → HDR10' : 'HDR10'} • 10-bit`);
    } else if (state.metadata.bitDepth > 8) {
      setStatus(`${state.metadata.width}×${state.metadata.height}${canvas.padded ? ` → ${canvas.width}×${canvas.height} with automatic lower subtitle area` : ''} • ${formatTime(state.metadata.duration)} • 10-bit SDR preserved`);
    } else {
      setStatus(`${state.metadata.width}×${state.metadata.height} • ${formatTime(state.metadata.duration)} • ${state.metadata.codec.toUpperCase()} • 8-bit source`);
    }
    if (state.subtitleWarning) setStatus(state.subtitleWarning, 'error');
    schedulePreview();
  } catch (error) {
    if (state.videoPath !== videoPath) return;
    state.metadata = null;
    updateEstimate();
    setStatus(String(error), 'error');
  }
  if (state.platform === 'windows') encoderPresetDescription();
  refreshReady();
  fitWindowToContent();
  // Auto-pick a matching subtitle, but never replace one the user chose by hand.
  if (state.metadata && (!state.subtitlePath || state.subtitleAuto)) {
    const found = await api.findSubtitle(videoPath).catch(() => '');
    if (found && state.videoPath === videoPath && (!state.subtitlePath || state.subtitleAuto)) {
      await setSubtitle(found, true);
      if (!state.subtitleWarning) setStatus(`Matched subtitles: ${basename(found)}`);
    }
  }
}

function updateSubtitleWarning() {
  state.subtitleWarning = '';
  if (!state.metadata?.duration || !state.subtitleTiming?.lastCueSeconds) return;
  if (state.subtitleTiming.lastCueSeconds < state.metadata.duration * 0.7) {
    state.subtitleWarning = `Warning: subtitles end at ${formatTime(state.subtitleTiming.lastCueSeconds)}, but the video lasts ${formatTime(state.metadata.duration)}. This is probably the wrong or incomplete subtitle file.`;
    setStatus(state.subtitleWarning, 'error');
  }
}

async function setSubtitle(subtitlePath, auto = false) {
  if (!subtitlePath) return;
  state.subtitleAuto = auto;
  if (state.mode === 'subtitles') {
    clearTimeout(state.previewTimer);
    state.previewRequest += 1;
    state.previewPending = null;
  }
  state.subtitlePath = subtitlePath;
  elements.subtitleName.textContent = basename(subtitlePath);
  const timing = await api.inspectSubtitle(subtitlePath).catch(() => null);
  if (state.subtitlePath !== subtitlePath) return;
  state.subtitleTiming = timing;
  updateSubtitleWarning();
  if (!state.subtitleWarning) setStatus(/\.ass$/i.test(subtitlePath) ? 'ASS styling will be preserved; only size and position are adjusted.' : 'Crunchyroll Trebuchet preset will be applied to this plain-text subtitle file.');
  if (state.mode === 'subtitles') schedulePreview();
  refreshReady();
  fitWindowToContent();
}

function releasePreviewFrame() {
  elements.previewFrame.removeAttribute('src');
  if (state.previewFrameUrl) URL.revokeObjectURL(state.previewFrameUrl);
  state.previewFrameUrl = null;
}

function previewFrameUrl(frame) {
  const contents = atob(frame.slice(frame.indexOf(',') + 1));
  const bytes = Uint8Array.from(contents, char => char.charCodeAt(0));
  return URL.createObjectURL(new Blob([bytes], { type: 'image/png' }));
}

async function renderPreview(reportStatus) {
  if (!(state.videoPath && state.metadata) || state.exporting) return;
  if (state.previewBusy) {
    state.previewPending = () => schedulePreview(reportStatus);
    return;
  }
  state.previewBusy = true;
  state.previewPending = null;
  const request = ++state.previewRequest;
  const renderedSettings = settings();
  const subtitlePath = state.mode === 'subtitles' ? state.subtitlePath || null : null;
  elements.previewSpinner.hidden = false;
  try {
    const result = await api.renderPreview({
      videoPath: state.videoPath, subtitlePath, metadata: state.metadata,
      settings: renderedSettings, time: Number(elements.seek.value),
    });
    if (request !== state.previewRequest) return;
    const url = previewFrameUrl(result.frame);
    result.frame = null;
    releasePreviewFrame();
    state.previewFrameUrl = url;
    elements.previewFrame.src = url;
    await elements.previewFrame.decode();
    if (request !== state.previewRequest) return;
    const { naturalWidth, naturalHeight } = elements.previewFrame;
    elements.previewStage.style.setProperty('--preview-ratio', naturalWidth / naturalHeight);
    elements.previewStage.classList.add('has-image');
    if (!reportStatus) return;
    if (result.subtitleError) setStatus(`Subtitle preview unavailable: ${result.subtitleError}`, 'error');
    else if (subtitlePath && state.subtitleWarning) setStatus(state.subtitleWarning, 'error');
    else if (!elements.status.classList.contains('success')) {
      const count = result.fontCount;
      setStatus(count ? `Preview ready • ${count} nearby font file${count === 1 ? '' : 's'} loaded.` : 'Preview ready.');
    }
  } catch (error) {
    if (request !== state.previewRequest) return;
    if (!elements.previewFrame.naturalWidth) {
      releasePreviewFrame();
      elements.previewStage.classList.remove('has-image');
    }
    if (reportStatus) setStatus(String(error), 'error');
  } finally {
    if (request === state.previewRequest) {
      elements.previewSpinner.hidden = true;
      state.previewDirty = false;
    }
    state.previewBusy = false;
    const pending = state.previewPending;
    state.previewPending = null;
    if (pending && !state.exporting) pending();
  }
}

function schedulePreview(reportStatus = true) {
  if (!state.videoPath || !state.metadata) return;
  state.previewDirty = true;
  if (state.exporting) return;
  clearTimeout(state.previewTimer);
  state.previewRequest += 1;
  state.previewPending = null;
  state.previewTimer = setTimeout(() => {
    state.previewTimer = null;
    renderPreview(reportStatus);
  }, 0);
}

restoreSubtitleSettings();
const sliderCaptions = [...document.querySelectorAll('.slider')].map(enhanceSlider);

elements.videoPicker.addEventListener('click', async () => setVideo(await api.chooseVideo()));
elements.subtitlePicker.addEventListener('click', async () => setSubtitle(await api.chooseSubtitle()));

elements.delay.addEventListener('input', () => { elements.delayValue.textContent = `${Number(elements.delay.value).toFixed(2)} s`; saveSubtitleSettings(); updateAutomaticOutput(); schedulePreview(); });
elements.position.addEventListener('input', () => {
  const value = Number(elements.position.value);
  elements.positionValue.textContent = value === 0 ? 'Top' : value === 50 ? 'Middle' : value === 100 ? 'Bottom' : `${value}% from top`;
  saveSubtitleSettings();
  updateAutomaticOutput();
  schedulePreview();
});
elements.scale.addEventListener('input', () => { elements.scaleValue.textContent = `${elements.scale.value}%`; saveSubtitleSettings(); updateAutomaticOutput(); schedulePreview(); });
elements.quality.addEventListener('input', () => {
  saveSubtitleSettings();
  updateAutomaticOutput();
  refreshQualityDisplay();
});
elements.windowsEncoder.addEventListener('change', () => { saveSubtitleSettings(); encoderPresetDescription(); syncEncoderChips(); updateEstimate(); });
elements.seek.addEventListener('input', () => { elements.seekCurrent.textContent = formatTime(elements.seek.value); schedulePreview(); });
elements.subtitleMode.addEventListener('click', () => setMode('subtitles'));
elements.compressMode.addEventListener('click', () => setMode('compress'));
const MAX_TARGET_MB = 250;
const targetRange = $('targetRange');
function syncTargetRange() {
  targetRange.value = String(Math.max(1, Math.min(MAX_TARGET_MB, Number(elements.targetSize.value) || 25)));
  targetRange.refreshSlider();
}
// Live estimates are independent of export readiness. Export validates the current target itself.
let estimateFrame = 0;
let estimateRequest = 0;
let estimatePending = false;
let estimateBusy = false;

function showEstimateStatus(note, error = false) {
  elements.estNote.textContent = note;
  elements.estNote.classList.toggle('error', error);
  elements.estimate.classList.toggle('bad', error);
}

function updateEstimate() {
  estimateRequest += 1;
  estimatePending = state.mode === 'compress' && Boolean(state.metadata);
  if (!estimatePending) {
    $('cpuNote').hidden = true;
    elements.estRes.textContent = '—';
    elements.estRate.textContent = '—';
    showEstimateStatus('Choose a video to see the estimate.');
    return;
  }
  queueEstimate();
}

function queueEstimate() {
  if (estimateBusy || estimateFrame || !estimatePending) return;
  estimateFrame = requestAnimationFrame(async () => {
    estimateFrame = 0;
    if (!estimatePending) return;
    const request = estimateRequest;
    const metadata = state.metadata;
    const target = Number(elements.targetSize.value);
    const encoder = elements.windowsEncoder.value;
    const codec = elements.compressionCodec.value;
    const av1 = codec !== 'h264' &&
      state.availableAv1Encoders.some(value => encoder === 'auto' || encoder === value);
    estimateBusy = true;
    try {
      const plan = await api.planCompression(metadata, target, av1, encoder);
      if (state.mode !== 'compress' || state.metadata !== metadata ||
          elements.windowsEncoder.value !== encoder || elements.compressionCodec.value !== codec) return;
      const rate = plan.videoBitrate >= 1e6
        ? `${(plan.videoBitrate / 1e6).toFixed(3)} Mbps`
        : `${Math.round(plan.videoBitrate / 1000)} kbps`;
      // Show intermediate results during a drag and apply notices for the latest target only.
      elements.estRes.textContent = `${plan.outputHeight}p`;
      elements.estRate.textContent = rate;
      if (request !== estimateRequest) return;
      estimatePending = false;
      $('cpuNote').hidden = !(plan.usesCpu && encoder !== 'software');
      showEstimateStatus(plan.bitrateCapped
        ? 'Bitrate is capped based on the source. A larger target won’t increase it.'
        : 'Source frame timing is preserved.');
    } catch (error) {
      if (request !== estimateRequest) return;
      estimatePending = false;
      $('cpuNote').hidden = true;
      elements.estRes.textContent = '—';
      elements.estRate.textContent = '—';
      showEstimateStatus(String(error), true);
    } finally {
      estimateBusy = false;
      queueEstimate();
    }
  });
}
targetRange.addEventListener('input', () => {
  elements.targetSize.value = targetRange.value;
  elements.targetSize.dispatchEvent(new Event('input', { bubbles: true }));
});
elements.targetSize.addEventListener('change', () => {
  const typed = Number(elements.targetSize.value);
  elements.targetSize.value = String(Number.isFinite(typed) && typed >= 1 ? Math.min(MAX_TARGET_MB, Math.round(typed)) : 25);
  elements.targetSize.dispatchEvent(new Event('input', { bubbles: true }));
});
elements.targetSize.addEventListener('input', () => {
  syncTargetRange();
  updateEstimate();
  if (state.mode !== 'compress') return;
  const target = Number(elements.targetSize.value);
  if (Number.isFinite(target) && target >= 1 && target <= MAX_TARGET_MB) localStorage.setItem(COMPRESSION_TARGET_KEY, String(target));
  updateAutomaticOutput();
});
elements.compressionCodec.addEventListener('change', () => {
  localStorage.setItem(COMPRESSION_CODEC_KEY, elements.compressionCodec.value);
  syncCodecChips();
  updateEstimate();
});
function stepTargetSize(delta) {
  const current = Number(elements.targetSize.value) || 25;
  elements.targetSize.value = String(Math.max(1, Math.min(MAX_TARGET_MB, current + delta)));
  elements.targetSize.dispatchEvent(new Event('input', { bubbles: true }));
}
elements.targetDown.addEventListener('click', () => stepTargetSize(-1));
elements.targetUp.addEventListener('click', () => stepTargetSize(1));

function beginExport() {
  clearTimeout(state.previewTimer);
  state.previewRequest += 1;
  elements.previewSpinner.hidden = true;
  state.exporting = true;
  refreshReady();
  elements.cancelButton.hidden = false;
  elements.progressWrap.hidden = false;
  elements.progressBar.style.width = '0%';
  elements.progressValue.textContent = '0%';
}

function finishExport() {
  state.exporting = false;
  elements.cancelButton.hidden = true;
  elements.progressWrap.hidden = true;
  if (state.previewDirty) schedulePreview(false);
  refreshReady();
  fitWindowToContent();
}

async function startCompression(isTest = false) {
  beginExport();
  const targetSizeMB = Number(elements.targetSize.value);
  const codecPreference = elements.compressionCodec.value;
  const clipDuration = isTest ? Math.min(30, state.metadata.duration) : 0;
  const clipStart = isTest ? Math.max(0, Math.min(Number(elements.seek.value), state.metadata.duration - clipDuration)) : 0;
  const outputPath = isTest ? `${dirname(state.videoPath)}${extless(state.videoPath)} — 30s compression test.mp4` : state.outputPath;
  const compressionEngine = state.platform === 'windows'
    ? 'Automatically testing NVIDIA, Intel, and AMD hardware encoders; CPU two-pass is the fallback.'
    : 'Using quality-first CPU two-pass compression.';
  setStatus(isTest ? `Compressing a 30-second test from ${formatTime(clipStart)} at the bitrate a ${targetSizeMB} MB file would use. ${compressionEngine}` : `Compressing to approximately ${targetSizeMB} MB with automatic bitrate and resolution. ${compressionEngine}`);
  fitWindowToContent();
  try {
    const result = await api.startCompression({
      videoPath: state.videoPath, outputPath, metadata: state.metadata, targetSizeMb: targetSizeMB, codecPreference, encoderPreference: elements.windowsEncoder.value, clipStart, clipDuration,
    });
    const actualMB = result.bytes / 1024 / 1024;
    const codecLabel = /^av1_/.test(result.encoder) ? 'AV1' : 'H.264';
    const encoderLabel = /nvenc/.test(result.encoder) ? `NVIDIA ${codecLabel} NVENC${result.nvencQp == null ? '' : ` CQP ${result.nvencQp}`}`
      : /qsv/.test(result.encoder) ? `Intel ${codecLabel} Quick Sync`
        : /amf/.test(result.encoder) ? `AMD ${codecLabel} AMF`
          : result.encoder === 'h264_videotoolbox' ? 'Apple VideoToolbox (H.264)'
            : 'CPU x264 two-pass';
    setStatus(`Finished with ${encoderLabel}: ${basename(result.output)} • ${actualMB.toFixed(1)} MB • ${result.metadata.width}×${result.metadata.height} — click here to show it.`, 'success');
    const output = result.output;
    elements.status.onclick = () => api.revealOutput(output);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const cancelled = message === 'Export cancelled.';
    setStatus(cancelled ? 'Compression cancelled.' : message, cancelled ? '' : 'error');
  } finally {
    finishExport();
  }
}

async function startBurn(isTest = false) {
  beginExport();
  const quality = Number(elements.quality.value);
  const clipDuration = isTest ? Math.min(30, state.metadata.duration) : 0;
  const clipStart = isTest ? Math.max(0, Math.min(Number(elements.seek.value), state.metadata.duration - clipDuration)) : 0;
  const outputPath = isTest ? `${dirname(state.videoPath)}${extless(state.videoPath)} — 30s test.mp4` : state.outputPath;
  const modeLabel = isTest ? `30-second test from ${formatTime(clipStart)}` : 'full video';
  const awakeNote = state.platform === 'macos' ? ' Your Mac will stay awake while encoding.' : '';
  const hardwareName = elements.windowsEncoder.value === 'software' ? 'CPU'
    : state.platform === 'windows' && elements.windowsEncoder.value === 'auto'
    ? autoEncoderLabel() || 'automatic encoding'
    : /videotoolbox/i.test(state.videoEncoder) ? 'Apple VideoToolbox' : 'the selected encoding settings';
  const burnDescription = state.metadata?.isHdr
    ? `Burning ${modeLabel} in 10-bit HDR with ${hardwareName} (CQ ${quality})…`
    : state.metadata?.bitDepth > 8
      ? `Burning ${modeLabel} in preserved 10-bit SDR with ${hardwareName} (CQ ${quality})…`
      : `Burning ${modeLabel} with libass and ${hardwareName} (CQ ${quality})…`;
  setStatus(burnDescription + awakeNote);
  fitWindowToContent();
  try {
    const result = await api.startExport({
      videoPath: state.videoPath, subtitlePath: state.subtitlePath, outputPath,
      metadata: state.metadata, settings: settings(), clipStart, clipDuration,
    });
    const encoderLabel = /_nvenc$/.test(result.encoder) ? `NVIDIA ${result.encoder.startsWith('hevc') ? 'HEVC NVENC' : 'H.264 NVENC'}`
      : /_amf$/.test(result.encoder) ? `AMD ${result.encoder.startsWith('hevc') ? 'HEVC AMF' : 'H.264 AMF'}`
        : /_qsv$/.test(result.encoder) ? `Intel ${result.encoder.startsWith('hevc') ? 'HEVC Quick Sync' : 'H.264 Quick Sync'}`
          : result.encoder === 'hevc_videotoolbox' ? `HEVC ${result.tenBit ? `Main10${result.hdr ? ' HDR' : ' 10-bit SDR'}` : 'Main'}`
            : result.encoder === 'h264_videotoolbox' ? 'Apple VideoToolbox' : result.encoder === 'libx265' ? 'HEVC Main10' : 'H.264';
    const hdrNote = result.dolbyVisionConverted ? ' • Dolby Vision converted safely to HDR10-compatible output' : '';
    const testNote = isTest ? `30-second test from ${formatTime(result.clipStart)} • ` : '';
    setStatus(`${testNote}Finished with ${encoderLabel}${hdrNote}: ${basename(result.output)} — click here to show it.`, 'success');
    const output = result.output;
    elements.status.onclick = () => api.revealOutput(output);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus(message, message === 'Export cancelled.' ? '' : 'error');
  } finally {
    finishExport();
  }
}
elements.testButton.addEventListener('click', () => state.mode === 'compress' ? startCompression(true) : startBurn(true));
elements.exportButton.addEventListener('click', () => state.mode === 'compress' ? startCompression() : startBurn(false));
elements.cancelButton.addEventListener('click', () => api.cancelExport());
elements.updateButton.addEventListener('click', () => state.availableUpdate ? installAvailableUpdate() : checkForUpdates(true));
api.onProgress(value => {
  if (!state.exporting) return;
  const percent = Math.max(0, Math.min(100, Math.round(value * 100)));
  elements.progressBar.style.width = `${percent}%`;
  elements.progressValue.textContent = `${percent}%`;
});

for (const target of [elements.videoPicker, elements.subtitlePicker]) {
  target.addEventListener('dragover', event => { event.preventDefault(); });
  target.addEventListener('drop', event => {
    event.preventDefault();
    const file = event.dataTransfer.files[0];
    if (!file) return;
    const filePath = api.filePath(file);
    if (target === elements.videoPicker) setVideo(filePath); else setSubtitle(filePath);
  });
}

const savedCompressionTarget = Number(localStorage.getItem(COMPRESSION_TARGET_KEY));
if (Number.isFinite(savedCompressionTarget) && savedCompressionTarget >= 1 && savedCompressionTarget <= 250) {
  elements.targetSize.value = String(savedCompressionTarget);
}
syncTargetRange();
syncEncoderChips();
syncCodecChips();
setMode('subtitles'); // always start on Burn
refreshQualityDisplay();
fitWindowToContent();
checkTools();
checkForUpdates();
window.addEventListener('load', fitWindowToContent, { once: true });
