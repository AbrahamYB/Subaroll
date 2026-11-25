const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

window.subaroll = {
  toolStatus: () => invoke('tool_status'),
  chooseFfmpeg: () => invoke('choose_ffmpeg'),
  chooseVideo: () => invoke('choose_video'),
  chooseSubtitle: () => invoke('choose_subtitle'),
  findSubtitle: videoPath => invoke('find_subtitle', { videoPath }),
  chooseOutput: suggested => invoke('choose_output', { suggested }),
  probe: videoPath => invoke('probe', { videoPath }),
  inspectSubtitle: subtitlePath => invoke('inspect_subtitle', { subtitlePath }),
  renderPreview: input => invoke('render_preview', { input }),
  startExport: input => invoke('start_export', { input }),
  planCompression: (metadata, targetSizeMB, av1, encoderPreference) => invoke('plan_compression', { metadata, targetSizeMb: targetSizeMB, av1, encoderPreference }),
  startCompression: input => invoke('start_compression', { input }),
  cancelExport: () => invoke('cancel_export'),
  revealOutput: outputPath => invoke('reveal_output', { outputPath }),
  checkForUpdate: () => invoke('check_for_update'),
  installUpdate: () => invoke('install_update'),
  fitWindow: () => {},
  filePath: file => file?.path || '',
  onProgress: callback => { listen('export-progress', event => callback(event.payload)); },
};
