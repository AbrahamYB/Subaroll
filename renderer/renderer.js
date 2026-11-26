const api = window.subaroll;
document.documentElement.classList.toggle('platform-windows', /Windows/.test(navigator.userAgent));
const state = {
  videoPath: '', subtitlePath: '', outputPath: '', metadata: null,
  mode: 'subtitles', exporting: false, previewTimer: null, previewRequest: 0, previewBusy: false, previewPending: null, previewDirty: false, previewFrameUrl: null, videoEncoder: '', platform: '', availableVideoEncoders: [], availableAv1Encoders: [], subtitleTiming: null, subtitleWarning: '',
  availableUpdate: null, updateBusy: false, autoVideoEncoder10bit: '',
  appleBurnAvailable: false, toolsReady: false, preferredEncoder: 'auto', preferredCodec: 'auto',
};

const $ = id => document.getElementById(id);
const elements = {
  videoPicker: $('videoPicker'), subtitlePicker: $('subtitlePicker'),
  videoName: $('videoName'), subtitleName: $('subtitleName'),
  delay: $('delay'), delayValue: $('delayValue'), position: $('position'), positionValue: $('positionValue'),
  scale: $('scale'), scaleValue: $('scaleValue'), seek: $('seek'), seekCurrent: $('seekCurrent'), seekDuration: $('seekDuration'),
  quality: $('quality'), qualityValue: $('qualityValue'), qualityHint: $('qualityHint'),
  windowsEncoder: $('windowsEncoder'),
  previewStage: $('previewStage'), previewFrame: $('previewFrame'), previewSpinner: $('previewSpinner'),
  exportButton: $('exportButton'), testButton: $('testButton'), cancelButton: $('cancelButton'), progressWrap: $('progressWrap'),
  progressValue: $('progressValue'), progressBar: $('progressBar'), status: $('status'),
  subtitleMode: $('subtitleMode'), compressMode: $('compressMode'),
  targetSize: $('targetSize'), estRes: $('estRes'), estFps: $('estFps'), estRate: $('estRate'), estNote: $('estNote'), estimate: $('estimate'), subtitleOnly: [...document.querySelectorAll('.subtitle-only')],
  compressionCodec: $('compressionCodec'),
  updateButton: $('updateButton'),
};

// The native range input handles keyboard and pointer interaction. The track marks snap points.
function enhanceSlider(box) {
  const input = box.querySelector('input[type=range]');
  const min = Number(input.min), max = Number(input.max), span = max - min;
  const pct = v => (v - min) / span * 100;
  const at = p => `calc(${p}% + ${(8 - 0.16 * p).toFixed(2)}px)`;
  const marks = (box.dataset.marks || '').split(';').filter(Boolean).map(m => m.split(':'));
  const snap = [...marks.map(m => Number(m[0])), ...(box.dataset.snap || '').split(',').filter(Boolean).map(Number)].sort((a, b) => a - b);
  const ns = document.createElement('div');
  ns.className = 'ns';
  ns.innerHTML = '<div class="tr"><div class="fl"></div></div><div class="th"></div>';
  const tr = ns.querySelector('.tr'), fill = ns.querySelector('.fl'), thumb = ns.querySelector('.th');
  const ticks = marks.map(([v]) => { const tick = document.createElement('div'); tick.className = 'tk'; tick.style.left = at(pct(Number(v))); tr.appendChild(tick); return tick; });
  const caption = document.createElement('div');
  caption.className = 'mk';
  marks.forEach(([v, text]) => { const label = document.createElement('span'); label.textContent = text; label.style.left = at(pct(Number(v))); caption.appendChild(label); });
  input.before(ns);
  ns.appendChild(input);
  box.appendChild(caption);

  const draw = () => {
    const value = Number(input.value);
    const p = pct(value);
    fill.style.width = at(p);
    thumb.style.left = at(p);
    const hit = snap.find(point => Math.abs(value - point) < 1e-6);
    ns.classList.toggle('lock', hit !== undefined);
    ticks.forEach((tick, i) => tick.classList.toggle('hit', Number(marks[i][0]) === hit));
  };
  // Keep the thumb under the pointer throughout the drag. Snap only on release when
  // the chosen value lands very close to a landmark.
  let dragging = false;
  input.addEventListener('input', draw);
  input.addEventListener('pointerdown', () => { dragging = true; });
  input.addEventListener('change', draw);
  const release = event => {
    if (!dragging) return;
    dragging = false;
    if (event.type === 'pointercancel') return;
    const radius = 3 * span / Math.max(16, ns.clientWidth - 16);
    const value = Number(input.value);
    const closest = snap.reduce((best, point) =>
      Math.abs(point - value) < Math.abs(best - value) ? point : best, Infinity);
    if (Math.abs(closest - value) <= radius && closest !== value) {
      input.value = String(closest);
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new Event('change', { bubbles: true }));
    }
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
    if (!caption.getClientRects().length) continue;
    let right = -Infinity;
    for (const label of caption.children) {
      label.style.visibility = '';
      label.style.removeProperty('--landmark-offset');
      if (caption.closest('#compressionOptions')) {
        const left = label.getBoundingClientRect().left;
        const offset = Math.max(0, right + 6 - left);
        label.style.setProperty('--landmark-offset', `${offset}px`);
      }
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
  localStorage.setItem(SETTINGS_KEY, JSON.stringify({ ...settings(), windowsEncoder: state.preferredEncoder, settingsVersion: 4 }));
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
    if (['auto', 'nvenc', 'amf', 'qsv', 'videotoolbox', 'software'].includes(saved.windowsEncoder)) state.preferredEncoder = saved.windowsEncoder;
    if (saved.settingsVersion !== 4) saveSubtitleSettings();
  } catch (_) {}
  elements.delayValue.textContent = `${Number(elements.delay.value).toFixed(2)} s`;
  displayPosition();
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
const encoderLabels = {
  nvenc: 'NVIDIA NVENC', qsv: 'Intel Quick Sync', amf: 'AMD AMF',
  videotoolbox: 'Apple VideoToolbox', software: 'CPU',
};

function availableAv1(encoder = elements.windowsEncoder.value) {
  return ['nvenc', 'qsv', 'amf'].some(key =>
    state.availableAv1Encoders.includes(key) && (encoder === 'auto' || encoder === key));
}

function autoEncoderLabel(usesCpu = false) {
  if (!state.toolsReady) return '';
  if (state.mode === 'compress') {
    if (usesCpu) return 'CPU';
    const order = ['nvenc', 'qsv', 'amf', 'videotoolbox'];
    const available = elements.compressionCodec.value !== 'h264' && availableAv1('auto')
      ? state.availableAv1Encoders : state.availableVideoEncoders;
    return encoderLabels[order.find(key => available.includes(key))] || 'CPU';
  }
  const tenBit = state.metadata?.isHdr || state.metadata?.bitDepth > 8;
  const encoder = (tenBit ? state.autoVideoEncoder10bit : state.videoEncoder) || '';
  const vendor = Object.keys(encoderLabels).find(key => encoder.endsWith('_' + key));
  if (state.platform === 'macos') return state.appleBurnAvailable ? encoderLabels.videotoolbox : 'CPU';
  return encoderLabels[vendor] || 'CPU';
}

function refreshAutoLabels(usesCpu = false) {
  const autoLabel = autoEncoderLabel(usesCpu);
  elements.windowsEncoder.options[0].textContent = autoLabel ? `Auto · ${autoLabel}` : 'Auto';
  elements.compressionCodec.options[0].textContent =
    availableAv1() && !usesCpu ? 'Auto · AV1' : 'Auto · H.264';
}

function refreshCodecOptions() {
  const select = elements.compressionCodec;
  const av1 = availableAv1();
  select.replaceChildren(new Option('Auto', 'auto'), new Option('H.264', 'h264'));
  if (av1) select.add(new Option('AV1', 'av1'), 1);
  select.value = state.preferredCodec === 'av1' && !av1 ? 'auto' : state.preferredCodec;
  refreshAutoLabels();
}

function setStatus(message, type = '') {
  elements.status.onclick = null;
  elements.status.textContent = message;
  elements.status.title = message;
  elements.status.className = `status ${type}`;
}
async function updateButtonMessage(message, tone) {
  const button = elements.updateButton;
  const animate = matchMedia('(prefers-reduced-motion: no-preference)').matches;
  const squash = async (from, to) => {
    if (!animate) return;
    const animation = button.animate(
      [{ transform: `scaleX(${from})` }, { transform: `scaleX(${to})` }],
      { duration: 180, easing: 'cubic-bezier(.4, 0, .2, 1)', fill: 'forwards' },
    );
    await animation.finished.catch(() => {});
    animation.cancel();
  };
  await squash(1, 0.82);
  button.textContent = message;
  button.classList.add(tone);
  await squash(0.82, 1);
  await new Promise(resolve => setTimeout(resolve, 2500));
  await squash(1, 0.82);
  button.textContent = 'Check for updates';
  button.classList.remove(tone);
  await squash(0.82, 1);
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
    } else {
      elements.updateButton.title = `Installed build ${installedVersion}. Check GitHub for a newer build.`;
      if (manual) await updateButtonMessage('Up to date', 'is-current');
      else elements.updateButton.textContent = 'Check for updates';
    }
  } catch (error) {
    elements.updateButton.title = `Update check failed: ${error}`;
    if (manual) await updateButtonMessage('Check failed', 'is-error');
    else {
      elements.updateButton.textContent = 'Check for updates';
      console.warn('Subaroll update check failed:', error);
    }
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
function displayPosition() {
  const value = Number(elements.position.value);
  elements.positionValue.textContent = value === 0 ? 'Top' : value === 50 ? 'Middle' : value === 100 ? 'Bottom' : `${value}% from top`;
}

// Fixed-size window: nothing to fit, only hide landmark labels that would collide.
function fitWindowToContent() { declutterCaptions(); }

function refreshReady() {
  const hasRequired = state.videoPath && state.outputPath && state.metadata &&
    (state.mode === 'compress' || state.subtitlePath);
  elements.exportButton.disabled = state.exporting || !hasRequired;
  elements.testButton.disabled = state.exporting || !hasRequired;
  elements.subtitleMode.disabled = state.exporting;
  elements.compressMode.disabled = state.exporting;
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
  elements.subtitleMode.setAttribute('aria-pressed', String(!compressing));
  elements.compressMode.setAttribute('aria-pressed', String(compressing));
  document.querySelectorAll('.compress-only').forEach(element => { element.hidden = !compressing; });
  elements.subtitleOnly.forEach(element => { element.hidden = compressing; });
  elements.exportButton.textContent = compressing ? 'Compress video' : 'Burn subtitles';
  updateAutomaticOutput();
  refreshEncoderAvailability();
  updateEstimate();
  setStatus('');
  refreshReady();
  fitWindowToContent();
  if (state.mode !== previousMode) {
    const dock = $('dock');
    dock.getAnimations().forEach(animation => animation.cancel());
    if (matchMedia('(prefers-reduced-motion: no-preference)').matches) {
      dock.animate(
        [{ opacity: 0.35, transform: 'translateY(-5px)' }, { opacity: 1, transform: 'translateY(0)' }],
        { duration: 330, easing: 'cubic-bezier(.18, .7, .22, 1)' },
      );
    }
    schedulePreview();
  }
}

function refreshEncoderAvailability() {
  const select = elements.windowsEncoder;
  const available = new Set(state.availableVideoEncoders);
  if (state.platform === 'macos' && state.mode === 'subtitles') {
    if (state.appleBurnAvailable) available.add('videotoolbox');
    else available.delete('videotoolbox');
  }
  select.replaceChildren(new Option('Auto', 'auto'));
  for (const key of ['nvenc', 'qsv', 'amf', 'videotoolbox', 'software']) {
    if (key === 'software' || available.has(key)) select.add(new Option(encoderLabels[key], key));
  }
  select.value = [...select.options].some(option => option.value === state.preferredEncoder)
    ? state.preferredEncoder : 'auto';
  select.disabled = !state.toolsReady;
  refreshCodecOptions();
}

async function checkTools() {
  try {
    const result = await api.toolStatus();
    state.platform = result.platform || '';
    document.documentElement.classList.toggle('platform-windows', state.platform === 'windows');
    document.documentElement.classList.toggle('platform-macos', state.platform === 'macos');
    state.toolsReady = result.ok;
    if (!result.ok) {
      setStatus('FFmpeg with libass is required.', 'error');
      return;
    }
    state.videoEncoder = result.videoEncoder;
    state.autoVideoEncoder10bit = result.autoVideoEncoder10bit;
    state.availableVideoEncoders = result.availableVideoEncoders || [];
    state.appleBurnAvailable = Boolean(result.appleBurnAvailable);
    state.availableAv1Encoders = result.availableAv1Encoders || [];
    const savedCodec = localStorage.getItem(COMPRESSION_CODEC_KEY) || localStorage.getItem(LEGACY_COMPRESSION_CODEC_KEY);
    if (['auto', 'av1', 'h264'].includes(savedCodec)) state.preferredCodec = savedCodec;
  } catch (error) {
    setStatus(`Encoder detection failed: ${error}`, 'error');
  } finally {
    refreshEncoderAvailability();
    updateEstimate();
  }
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
  elements.seek.classList.remove('preview-loaded');
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
  if (state.mode === 'subtitles') refreshAutoLabels();
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
    elements.previewStage.classList.add('has-image');
    if (elements.seek.disabled && state.metadata.duration > 0) {
      elements.seek.disabled = false;
      elements.seek.classList.add('preview-loaded');
    }
    if (!reportStatus) return;
    if (result.subtitleError) setStatus(`Subtitle preview unavailable: ${result.subtitleError}`, 'error');
    else if (subtitlePath && state.subtitleWarning) setStatus(state.subtitleWarning, 'error');
  } catch (error) {
    if (request !== state.previewRequest) return;
    if (!elements.previewFrame.naturalWidth) {
      releasePreviewFrame();
      elements.previewStage.classList.remove('has-image');
      elements.seek.disabled = true;
      elements.seek.classList.remove('preview-loaded');
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
$('emptyVideoPicker').addEventListener('click', () => elements.videoPicker.click());
elements.subtitlePicker.addEventListener('click', async () => setSubtitle(await api.chooseSubtitle()));

elements.delay.addEventListener('input', () => { elements.delayValue.textContent = `${Number(elements.delay.value).toFixed(2)} s`; saveSubtitleSettings(); updateAutomaticOutput(); schedulePreview(); });
elements.position.addEventListener('input', () => {
  displayPosition();
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
elements.windowsEncoder.addEventListener('change', () => {
  state.preferredEncoder = elements.windowsEncoder.value;
  saveSubtitleSettings();
  refreshCodecOptions();
  updateEstimate();
});
elements.seek.addEventListener('input', () => { elements.seekCurrent.textContent = formatTime(elements.seek.value); schedulePreview(); });
elements.subtitleMode.addEventListener('click', () => setMode('subtitles'));
elements.compressMode.addEventListener('click', () => setMode('compress'));
const MAX_TARGET_MB = Number(elements.targetSize.max);
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

function showEstimateStatus(note, error = false, notice = false) {
  elements.estNote.textContent = note;
  elements.estNote.classList.toggle('error', error);
  elements.estNote.classList.toggle('notice', notice);
  elements.estimate.classList.toggle('bad', error);
}

function updateEstimate() {
  estimateRequest += 1;
  estimatePending = state.mode === 'compress' && Boolean(state.metadata);
  if (!estimatePending) {
    elements.estRes.textContent = '—';
    elements.estRate.textContent = '—';
    showEstimateStatus('Choose a video to see the estimate.');
    return;
  }
  queueEstimate();
}

function formatVideoBitrate(bitsPerSecond) {
  return bitsPerSecond >= 1e6
    ? `${(bitsPerSecond / 1e6).toFixed(3)} Mbps`
    : `${Math.round(bitsPerSecond / 1000)} kbps`;
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
    const av1 = codec !== 'h264' && availableAv1(encoder);
    estimateBusy = true;
    try {
      const plan = await api.planCompression(metadata, target, av1, encoder);
      if (state.mode !== 'compress' || state.metadata !== metadata ||
          elements.windowsEncoder.value !== encoder || elements.compressionCodec.value !== codec) return;
      // Show intermediate results during a drag and apply notices for the latest target only.
      elements.estRes.textContent = `${plan.outputHeight}p`;
      elements.estRate.textContent = formatVideoBitrate(plan.videoBitrate);
      if (request !== estimateRequest) return;
      estimatePending = false;
      refreshAutoLabels(plan.usesCpu);
      const notices = [];
      if (plan.bitrateCapped) notices.push('A larger target won’t increase the bitrate for this source.');
      if (plan.usesCpu && encoder !== 'software') notices.push('This target uses CPU encoding.');
      showEstimateStatus(notices.join(' '), false, notices.length > 0);
    } catch (error) {
      if (request !== estimateRequest) return;
      estimatePending = false;
      refreshAutoLabels();
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
  state.preferredCodec = elements.compressionCodec.value;
  localStorage.setItem(COMPRESSION_CODEC_KEY, state.preferredCodec);
  refreshAutoLabels();
  updateEstimate();
});

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

function exportSample(isTest) {
  if (!isTest || state.metadata.duration <= 30) return { clipStart: 0, clipDuration: 0 };
  return {
    clipStart: Math.max(0, Math.min(Number(elements.seek.value), state.metadata.duration - 30)),
    clipDuration: 30,
  };
}

async function startCompression(isTest = false) {
  beginExport();
  const targetSizeMB = Number(elements.targetSize.value);
  const codecPreference = elements.compressionCodec.value;
  const { clipStart, clipDuration } = exportSample(isTest);
  const outputPath = isTest ? `${dirname(state.videoPath)}${extless(state.videoPath)} — 30s compression test.mp4` : state.outputPath;
  setStatus(clipDuration
    ? `Compressing a 30-second test from ${formatTime(clipStart)} at the planned bitrate for ${targetSizeMB} MB.`
    : `Compressing the ${isTest ? 'full-length test' : 'full video'} to fit ${targetSizeMB} MB.`);
  fitWindowToContent();
  try {
    const result = await api.startCompression({
      videoPath: state.videoPath, outputPath, metadata: state.metadata, targetSizeMb: targetSizeMB, codecPreference, encoderPreference: elements.windowsEncoder.value, clipStart, clipDuration,
    });
    const actualMB = result.bytes / (targetSizeMB >= 5 ? 1_000_000 : 1_048_576);
    const codecLabel = /^av1_/.test(result.encoder) ? 'AV1' : 'H.264';
    const encoderLabel = /nvenc/.test(result.encoder) ? `NVIDIA ${codecLabel} NVENC${result.nvencQp == null ? '' : ` CQP ${result.nvencQp}`}`
      : /qsv/.test(result.encoder) ? `Intel ${codecLabel} Quick Sync`
        : /amf/.test(result.encoder) ? `AMD ${codecLabel} AMF`
          : result.encoder === 'h264_videotoolbox' ? 'Apple VideoToolbox (H.264)'
            : 'CPU x264 two-pass';
    const actualRate = result.metadata.bitRate > 0
      ? ` • ${formatVideoBitrate(result.metadata.bitRate)} video` : '';
    setStatus(`Finished with ${encoderLabel}: ${actualMB.toFixed(1)} MB • ${result.metadata.width}×${result.metadata.height}${actualRate} — click here to show the file.`, 'success');
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
  const { clipStart, clipDuration } = exportSample(isTest);
  const outputPath = isTest ? `${dirname(state.videoPath)}${extless(state.videoPath)} — 30s test.mp4` : state.outputPath;
  const modeLabel = clipDuration ? `30-second test from ${formatTime(clipStart)}` : isTest ? 'full-length test' : 'full video';
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
            : result.encoder === 'h264_videotoolbox' ? 'Apple VideoToolbox' : result.encoder === 'libx265' ? (result.tenBit ? 'HEVC Main10' : 'HEVC Main') : 'H.264';
    const hdrNote = result.dolbyVisionConverted ? ' • Dolby Vision converted safely to HDR10-compatible output' : '';
    const testNote = isTest ? `${modeLabel} • ` : '';
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
if (Number.isFinite(savedCompressionTarget) && savedCompressionTarget >= 1 && savedCompressionTarget <= MAX_TARGET_MB) {
  elements.targetSize.value = String(savedCompressionTarget);
}
syncTargetRange();
setMode('subtitles'); // always start on Burn
refreshQualityDisplay();
fitWindowToContent();
checkTools();
checkForUpdates();
window.addEventListener('load', fitWindowToContent, { once: true });
window.addEventListener('resize', fitWindowToContent);
