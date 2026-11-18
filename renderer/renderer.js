const api = window.subaroll;
const state = {
  videoPath: '', subtitlePath: '', outputPath: '', metadata: null,
  mode: 'subtitles', exporting: false, previewTimer: null, previewRequest: 0, previewBusy: false, previewPending: false, videoEncoder: '', platform: '', availableVideoEncoders: [], availableAv1Encoders: [], renderedPreviewSettings: null, renderedPreviewLayout: null, subtitleTiming: null, subtitleWarning: '',
  availableUpdate: null, updateBusy: false,
};

const $ = id => document.getElementById(id);
const elements = {
  videoPicker: $('videoPicker'), subtitlePicker: $('subtitlePicker'),
  videoName: $('videoName'), subtitleName: $('subtitleName'),
  delay: $('delay'), delayValue: $('delayValue'), position: $('position'), positionValue: $('positionValue'),
  scale: $('scale'), scaleValue: $('scaleValue'), seek: $('seek'), seekCurrent: $('seekCurrent'), seekDuration: $('seekDuration'),
  quality: $('quality'), qualityValue: $('qualityValue'), qualityHint: $('qualityHint'),
  windowsEncoderCard: $('windowsEncoderCard'), windowsEncoder: $('windowsEncoder'), windowsPreset: $('windowsPreset'),
  encoderAvailability: $('encoderAvailability'), encoderHint: $('encoderHint'),
  previewStage: $('previewStage'), previewFrame: $('previewFrame'), previewSubtitles: $('previewSubtitles'), previewSpinner: $('previewSpinner'),
  exportButton: $('exportButton'), testButton: $('testButton'), cancelButton: $('cancelButton'), progressWrap: $('progressWrap'),
  progressValue: $('progressValue'), progressBar: $('progressBar'), status: $('status'),
  subtitleMode: $('subtitleMode'), compressMode: $('compressMode'), compressionOptions: $('compressionOptions'),
  targetSize: $('targetSize'), previewNote: document.querySelector('.preview-note'), subtitleOnly: [...document.querySelectorAll('.subtitle-only')],
  targetDown: $('targetDown'), targetUp: $('targetUp'),
  compressionCodec: $('compressionCodec'), compressionCodecHint: $('compressionCodecHint'),
  modeEyebrow: $('modeEyebrow'), modeTitle: $('modeTitle'),
  updateButton: $('updateButton'),
};

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
    windowsEncoder: elements.windowsEncoder.value, windowsPreset: elements.windowsPreset.value,
  };
}
const SETTINGS_KEY = 'subaroll.subtitleSettings';
const LEGACY_SETTINGS_KEY = 'subtitleBurner.subtitleSettings';
const COMPRESSION_CODEC_KEY = 'subaroll.compressionCodec';
const LEGACY_COMPRESSION_CODEC_KEY = 'subtitleBurner.compressionCodec';
function saveSubtitleSettings() {
  localStorage.setItem(SETTINGS_KEY, JSON.stringify({ ...settings(), settingsVersion: 4 }));
}
function restoreSubtitleSettings() {
  try {
    const currentSettings = localStorage.getItem(SETTINGS_KEY);
    const existing = currentSettings || localStorage.getItem(LEGACY_SETTINGS_KEY);
    const legacyQuality = Number(localStorage.getItem('subtitleBurner.encodingQuality'));
    const saved = existing ? JSON.parse(existing) : {
      delaySeconds: 0,
      positionPercent: 95,
      scalePercent: 115,
      quality: Number.isFinite(legacyQuality) ? legacyQuality : 90,
    };
    if (Number.isFinite(saved.delaySeconds)) elements.delay.value = String(Math.max(-5, Math.min(5, saved.delaySeconds)));
    if (Number.isFinite(saved.positionPercent)) {
      const position = saved.settingsVersion === 2 ? saved.positionPercent : 95 + (saved.positionPercent - 100);
      elements.position.value = String(Math.max(0, Math.min(100, position)));
    }
    if (Number.isFinite(saved.scalePercent)) elements.scale.value = String(Math.max(50, Math.min(180, saved.scalePercent)));
    if (Number.isFinite(saved.quality)) elements.quality.value = String(Math.max(60, Math.min(100, saved.quality)));
    if (['auto', 'nvenc', 'amf', 'qsv', 'software'].includes(saved.windowsEncoder)) elements.windowsEncoder.value = saved.windowsEncoder;
    if (['quality', 'balanced', 'fast'].includes(saved.windowsPreset)) elements.windowsPreset.value = saved.windowsPreset;
    if (!currentSettings) saveSubtitleSettings();
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
  elements.qualityValue.textContent = `CQ ${value} · ${label}`;
  elements.qualityHint.textContent = hint;
}
function encoderPresetDescription() {
  const encoder = elements.windowsEncoder.value;
  const preset = elements.windowsPreset.value;
  const names = { auto: 'Auto', nvenc: 'NVIDIA NVENC', amf: 'AMD AMF', qsv: 'Intel Quick Sync', software: 'CPU' };
  const speeds = { quality: 'CTT quality', balanced: 'balanced', fast: 'fast' };
  elements.encoderHint.textContent = `${names[encoder]} · ${speeds[preset]}. Codec, 10-bit, HDR, resolution, and color metadata remain source-aware.`;
}
function setStatus(message, type = '') { elements.status.textContent = message; elements.status.className = `status ${type}`; }
async function checkForUpdates(manual = false) {
  if (state.updateBusy) return;
  state.updateBusy = true;
  elements.updateButton.disabled = true;
  elements.updateButton.textContent = 'Checking…';
  try {
    const update = await api.checkForUpdate();
    state.availableUpdate = update;
    if (update) {
      elements.updateButton.textContent = `Install v${publicReleaseVersion(update.version)}`;
      elements.updateButton.title = update.notes || 'A new Subaroll update is available.';
      if (manual) setStatus(`Subaroll ${publicReleaseVersion(update.version)} is ready to install.`);
    } else {
      elements.updateButton.textContent = 'Check for updates';
      elements.updateButton.title = 'Check GitHub for a newer Subaroll release.';
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
let fitTimer = null;
function fitWindowToContent() {
  clearTimeout(fitTimer);
  // Keep the native window stable. Programmatic resizing after paint can
  // invalidate Electron compositor frames on macOS.
}
function refreshReady() {
  const hasRequired = state.mode === 'compress'
    ? state.videoPath && state.outputPath && state.metadata && Number(elements.targetSize.value) > 0
    : state.videoPath && state.subtitlePath && state.outputPath && state.metadata;
  elements.exportButton.disabled = state.exporting || !hasRequired;
  elements.testButton.disabled = state.exporting || state.mode !== 'subtitles' || !(state.videoPath && state.subtitlePath && state.metadata);
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
  state.mode = mode === 'compress' ? 'compress' : 'subtitles';
  const compressing = state.mode === 'compress';
  elements.subtitleMode.classList.toggle('active', !compressing);
  elements.compressMode.classList.toggle('active', compressing);
  elements.compressionOptions.hidden = !compressing;
  elements.subtitleOnly.forEach(element => { element.hidden = compressing; });
  elements.previewNote.textContent = compressing
    ? 'Compression keeps the original aspect ratio and chooses bitrate and resolution for the target size.'
    : 'Position and size update instantly. The subtitle layer is rendered by libass.';
  elements.exportButton.textContent = compressing ? 'Compress video' : 'Burn subtitles';
  elements.modeEyebrow.textContent = compressing ? 'TARGET-SIZE COMPRESSION' : 'LIBASS BURN-IN';
  elements.modeTitle.textContent = compressing ? 'Compress video' : 'Burn subtitles';
  updateAutomaticOutput();
  setStatus(compressing ? 'Choose a video and target size. The compressed file saves beside the source automatically.' : 'Subtitle burn-in mode ready.');
  refreshReady();
  fitWindowToContent();
}

async function checkTools() {
  const result = await api.toolStatus();
  state.platform = result.platform || '';
  const windows = state.platform === 'windows';
  document.documentElement.classList.toggle('platform-windows', windows);
  elements.windowsEncoderCard.classList.toggle('is-unavailable', !windows);
  elements.windowsEncoder.disabled = !windows;
  elements.windowsPreset.disabled = !windows;
  elements.encoderAvailability.textContent = windows ? 'Detecting…' : 'Windows only';
  if (result.ok) {
    state.videoEncoder = result.videoEncoder;
    state.availableVideoEncoders = result.availableVideoEncoders || [];
    state.availableAv1Encoders = result.availableAv1Encoders || [];
    const av1Option = elements.compressionCodec.querySelector('option[value="av1"]');
    av1Option.disabled = state.availableAv1Encoders.length === 0;
    const savedCodec = localStorage.getItem(COMPRESSION_CODEC_KEY) || localStorage.getItem(LEGACY_COMPRESSION_CODEC_KEY);
    if (['auto', 'av1', 'h264'].includes(savedCodec)) elements.compressionCodec.value = savedCodec;
    if (savedCodec && !localStorage.getItem(COMPRESSION_CODEC_KEY)) localStorage.setItem(COMPRESSION_CODEC_KEY, savedCodec);
    if (elements.compressionCodec.selectedOptions[0]?.disabled) elements.compressionCodec.value = 'auto';
    const av1Labels = { nvenc: 'NVIDIA NVENC', qsv: 'Intel Quick Sync', amf: 'AMD AMF' };
    elements.compressionCodecHint.textContent = state.availableAv1Encoders.length
      ? `Hardware AV1 ready: ${state.availableAv1Encoders.map(value => av1Labels[value]).join(' + ')}.`
      : 'No usable hardware AV1 encoder detected; Auto uses H.264 or CPU fallback.';
    if (windows) {
      const available = new Set(state.availableVideoEncoders);
      elements.windowsEncoder.disabled = false;
      elements.windowsPreset.disabled = false;
      for (const option of elements.windowsEncoder.options) {
        if (!['auto', 'software'].includes(option.value)) option.disabled = !available.has(option.value);
      }
      if (elements.windowsEncoder.selectedOptions[0]?.disabled) elements.windowsEncoder.value = 'auto';
      const labels = { nvenc: 'NVIDIA', amf: 'AMD', qsv: 'Intel' };
      elements.encoderAvailability.textContent = state.availableVideoEncoders.length
        ? state.availableVideoEncoders.map(value => labels[value]).join(' + ') + ' detected'
        : 'CPU only';
      encoderPresetDescription();
      saveSubtitleSettings();
    }
  } else {
    elements.encoderAvailability.textContent = windows ? 'Unavailable' : 'Windows only';
    setStatus('FFmpeg with libass is required.', 'error');
  }
}

async function setVideo(videoPath) {
  if (!videoPath) return;
  clearTimeout(state.previewTimer);
  state.previewRequest += 1;
  state.previewPending = false;
  state.videoPath = videoPath;
  elements.videoName.textContent = basename(videoPath);
  setStatus('Reading video details…');
  try {
    state.metadata = await api.probe(videoPath);
    const canvas = previewCanvas(state.metadata);
    elements.previewStage.style.aspectRatio = `${canvas.width} / ${canvas.height}`;
    elements.seek.max = String(state.metadata.duration || 0);
    elements.seek.value = String(Math.min(30, Math.max(0, (state.metadata.duration || 0) * .08)));
    elements.seek.disabled = !state.metadata.duration;
    elements.seekDuration.textContent = formatTime(state.metadata.duration);
    elements.seekCurrent.textContent = formatTime(elements.seek.value);
    updateAutomaticOutput();
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
    state.metadata = null;
    setStatus(error.message, 'error');
  }
  refreshReady();
  fitWindowToContent();
}

function updateSubtitleWarning() {
  state.subtitleWarning = '';
  if (!state.metadata?.duration || !state.subtitleTiming?.lastCueSeconds) return;
  if (state.subtitleTiming.lastCueSeconds < state.metadata.duration * 0.7) {
    state.subtitleWarning = `Warning: subtitles end at ${formatTime(state.subtitleTiming.lastCueSeconds)}, but the video lasts ${formatTime(state.metadata.duration)}. This is probably the wrong or incomplete subtitle file.`;
    setStatus(state.subtitleWarning, 'error');
  }
}

async function setSubtitle(subtitlePath) {
  if (!subtitlePath) return;
  clearTimeout(state.previewTimer);
  state.previewRequest += 1;
  state.previewPending = false;
  state.subtitlePath = subtitlePath;
  elements.subtitleName.textContent = basename(subtitlePath);
  try { state.subtitleTiming = await api.inspectSubtitle(subtitlePath); } catch (_) { state.subtitleTiming = null; }
  updateSubtitleWarning();
  if (!state.subtitleWarning) setStatus(/\.ass$/i.test(subtitlePath) ? 'ASS styling will be preserved; only size and position are adjusted.' : 'Crunchyroll Trebuchet preset will be applied to this plain-text subtitle file.');
  schedulePreview();
  refreshReady();
  fitWindowToContent();
}

async function renderPreview() {
  if (!(state.videoPath && state.subtitlePath && state.metadata) || state.exporting) return;
  if (state.previewBusy) {
    state.previewPending = true;
    return;
  }
  state.previewBusy = true;
  state.previewPending = false;
  const request = ++state.previewRequest;
  const renderedSettings = settings();
  elements.previewSpinner.hidden = false;
  try {
    const result = await api.renderPreview({
      videoPath: state.videoPath, subtitlePath: state.subtitlePath, metadata: state.metadata,
      settings: renderedSettings, time: Number(elements.seek.value),
    });
    if (request !== state.previewRequest) return;
    elements.previewFrame.onload = () => { elements.previewSpinner.hidden = true; elements.previewStage.classList.add('has-image'); };
    elements.previewFrame.onerror = () => {
      if (request !== state.previewRequest) return;
      elements.previewSpinner.hidden = true;
      elements.previewStage.classList.remove('has-image');
      setStatus('The rendered preview image could not be displayed.', 'error');
    };
    state.renderedPreviewSettings = renderedSettings;
    state.renderedPreviewLayout = result.layout;
    elements.previewSubtitles.onload = () => {
      if (request !== state.previewRequest) return;
      elements.previewSubtitles.style.visibility = 'visible';
      applyLiveSubtitleTransform();
    };
    elements.previewFrame.src = result.frame;
    elements.previewSubtitles.src = result.subtitles;
    elements.previewSubtitles.style.transform = 'none';
    if (state.subtitleWarning) setStatus(state.subtitleWarning, 'error');
    else setStatus(result.fontCount ? `Preview ready • ${result.fontCount} nearby font file${result.fontCount === 1 ? '' : 's'} loaded.` : 'Preview ready.');
  } catch (error) {
    if (request !== state.previewRequest) return;
    elements.previewSpinner.hidden = true;
    elements.previewSubtitles.style.visibility = 'visible';
    setStatus(error.message, 'error');
  } finally {
    state.previewBusy = false;
    if (state.previewPending && !state.exporting) {
      state.previewPending = false;
      schedulePreview(0);
    }
  }
}

function applyLiveSubtitleTransform() {
  if (!state.renderedPreviewSettings || !state.renderedPreviewLayout || !elements.previewStage.classList.contains('has-image')) return;
  const currentPosition = Number(elements.position.value);
  const currentScale = Number(elements.scale.value);
  const { playResY, lineCount } = state.renderedPreviewLayout;
  const anchor = (position, scalePercent) => {
    const topRoom = Math.min(playResY * 0.3, 4 + 30 * (scalePercent / 100) * lineCount);
    return (topRoom + (playResY - 4 - topRoom) * position / 100) / playResY;
  };
  const renderedAnchor = anchor(state.renderedPreviewSettings.positionPercent, state.renderedPreviewSettings.scalePercent);
  const currentAnchor = anchor(currentPosition, currentScale);
  const positionDelta = (currentAnchor - renderedAnchor) * 100;
  const scaleRatio = currentScale / state.renderedPreviewSettings.scalePercent;
  elements.previewSubtitles.style.transformOrigin = `50% ${currentAnchor * 100}%`;
  elements.previewSubtitles.style.transform = `translate3d(0, ${positionDelta}%, 0) scale(${scaleRatio})`;
}

function beginLiveSubtitleAdjustment() {
  clearTimeout(state.previewTimer);
  elements.previewSpinner.hidden = true;
  applyLiveSubtitleTransform();
}

function schedulePreview(delay = 320) {
  clearTimeout(state.previewTimer);
  state.previewRequest += 1;
  state.previewTimer = setTimeout(renderPreview, delay);
}

elements.videoPicker.addEventListener('click', async () => setVideo(await api.chooseVideo()));
elements.subtitlePicker.addEventListener('click', async () => setSubtitle(await api.chooseSubtitle()));

elements.delay.addEventListener('input', () => { elements.delayValue.textContent = `${Number(elements.delay.value).toFixed(2)} s`; saveSubtitleSettings(); updateAutomaticOutput(); schedulePreview(); });
elements.position.addEventListener('input', () => {
  const value = Number(elements.position.value);
  elements.positionValue.textContent = value === 0 ? 'Top' : value === 50 ? 'Middle' : value === 100 ? 'Bottom' : `${value}% from top`;
  saveSubtitleSettings();
  updateAutomaticOutput();
  beginLiveSubtitleAdjustment();
});
elements.scale.addEventListener('input', () => { elements.scaleValue.textContent = `${elements.scale.value}%`; saveSubtitleSettings(); updateAutomaticOutput(); beginLiveSubtitleAdjustment(); });
elements.quality.addEventListener('input', () => {
  saveSubtitleSettings();
  updateAutomaticOutput();
  refreshQualityDisplay();
});
elements.windowsEncoder.addEventListener('change', () => { saveSubtitleSettings(); encoderPresetDescription(); });
elements.windowsPreset.addEventListener('change', () => { saveSubtitleSettings(); encoderPresetDescription(); });
elements.seek.addEventListener('input', () => { elements.seekCurrent.textContent = formatTime(elements.seek.value); schedulePreview(220); });
elements.subtitleMode.addEventListener('click', () => setMode('subtitles'));
elements.compressMode.addEventListener('click', () => setMode('compress'));
elements.targetSize.addEventListener('input', () => {
  if (state.mode !== 'compress') return;
  updateAutomaticOutput();
  refreshReady();
});
elements.compressionCodec.addEventListener('change', () => {
  localStorage.setItem(COMPRESSION_CODEC_KEY, elements.compressionCodec.value);
});
function stepTargetSize(delta) {
  const current = Number(elements.targetSize.value) || 25;
  elements.targetSize.value = String(Math.max(1, Math.min(10000, current + delta)));
  elements.targetSize.dispatchEvent(new Event('input', { bubbles: true }));
}
elements.targetDown.addEventListener('click', () => stepTargetSize(-1));
elements.targetUp.addEventListener('click', () => stepTargetSize(1));

async function startCompression() {
  clearTimeout(state.previewTimer);
  state.previewRequest += 1;
  state.exporting = true;
  refreshReady();
  elements.cancelButton.hidden = false;
  elements.progressWrap.hidden = false;
  elements.progressBar.style.width = '0%';
  elements.progressValue.textContent = '0%';
  const targetSizeMB = Number(elements.targetSize.value);
  const codecPreference = elements.compressionCodec.value;
  const compressionEngine = state.platform === 'windows'
    ? 'Automatically testing NVIDIA, Intel, and AMD hardware encoders; CPU two-pass is the fallback.'
    : 'Using quality-first CPU two-pass compression.';
  setStatus(`Compressing to approximately ${targetSizeMB} MB with automatic bitrate and resolution. ${compressionEngine}`);
  fitWindowToContent();
  try {
    const result = await api.startCompression({
      videoPath: state.videoPath, outputPath: state.outputPath, metadata: state.metadata, targetSizeMB, codecPreference,
    });
    const actualMB = result.bytes / 1024 / 1024;
    const codecLabel = /^av1_/.test(result.encoder) ? 'AV1' : 'H.264';
    const encoderLabel = /nvenc/.test(result.encoder) ? `NVIDIA ${codecLabel} NVENC CQP ${result.nvencQp || 20}`
      : /qsv/.test(result.encoder) ? `Intel ${codecLabel} Quick Sync`
        : /amf/.test(result.encoder) ? `AMD ${codecLabel} AMF`
          : 'CPU x264 two-pass';
    setStatus(`Finished with ${encoderLabel}: ${basename(result.output)} • ${actualMB.toFixed(1)} MB • ${result.metadata.width}×${result.metadata.height} — click here to show it.`, 'success');
    elements.status.onclick = () => api.revealOutput(result.output);
  } catch (error) {
    setStatus(error.message.includes('SIGTERM') ? 'Compression cancelled.' : error.message, error.message.includes('SIGTERM') ? '' : 'error');
  } finally {
    state.exporting = false;
    elements.cancelButton.hidden = true;
    refreshReady();
    fitWindowToContent();
  }
}

async function startBurn(isTest = false) {
  clearTimeout(state.previewTimer);
  state.previewRequest += 1;
  state.exporting = true;
  refreshReady();
  elements.cancelButton.hidden = false;
  elements.progressWrap.hidden = false;
  elements.progressBar.style.width = '0%';
  elements.progressValue.textContent = '0%';
  const quality = Number(elements.quality.value);
  const clipDuration = isTest ? Math.min(30, state.metadata.duration) : 0;
  const clipStart = isTest ? Math.max(0, Math.min(Number(elements.seek.value), state.metadata.duration - clipDuration)) : 0;
  const outputPath = isTest ? `${dirname(state.videoPath)}${extless(state.videoPath)} — 30s test.mp4` : state.outputPath;
  const modeLabel = isTest ? `30-second test from ${formatTime(clipStart)}` : 'full video';
  const awakeNote = ' Your Mac will stay awake if the screen locks or turns off.';
  const hardwareName = /nvenc/i.test(state.videoEncoder) ? 'NVIDIA NVENC'
    : /_amf/i.test(state.videoEncoder) ? 'AMD AMF'
      : /_qsv/i.test(state.videoEncoder) ? 'Intel Quick Sync'
        : /videotoolbox/i.test(state.videoEncoder) ? 'Apple VideoToolbox'
          : 'the best available encoder';
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
    elements.status.onclick = () => api.revealOutput(result.output);
  } catch (error) {
    setStatus(error.message.includes('SIGTERM') ? 'Export cancelled.' : error.message, error.message.includes('SIGTERM') ? '' : 'error');
  } finally {
    state.exporting = false;
    elements.cancelButton.hidden = true;
    refreshReady();
    fitWindowToContent();
  }
}
elements.testButton.addEventListener('click', () => startBurn(true));
elements.exportButton.addEventListener('click', () => state.mode === 'compress' ? startCompression() : startBurn(false));
elements.cancelButton.addEventListener('click', () => api.cancelExport());
elements.updateButton.addEventListener('click', () => state.availableUpdate ? installAvailableUpdate() : checkForUpdates(true));
api.onProgress(value => {
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

restoreSubtitleSettings();
refreshQualityDisplay();
checkTools();
checkForUpdates();
window.addEventListener('load', fitWindowToContent, { once: true });
