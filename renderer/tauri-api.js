const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

window.subtitleBurner = {
  toolStatus: () => invoke('tool_status'),
  chooseFfmpeg: () => invoke('choose_ffmpeg'),
  chooseVideo: () => invoke('choose_video'),
  chooseSubtitle: () => invoke('choose_subtitle'),
  chooseOutput: suggested => invoke('choose_output', { suggested }),
  probe: videoPath => invoke('probe', { videoPath }),
  inspectSubtitle: subtitlePath => invoke('inspect_subtitle', { subtitlePath }),
  renderPreview: input => invoke('render_preview', { input }),
  startExport: input => invoke('start_export', { input }),
  startCompression: input => invoke('start_compression', { input }),
  cancelExport: () => invoke('cancel_export'),
  revealOutput: outputPath => invoke('reveal_output', { outputPath }),
  fitWindow: () => {},
  filePath: file => file?.path || '',
  onProgress: callback => { listen('export-progress', event => callback(event.payload)); },
};
