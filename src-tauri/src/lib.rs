mod media;

use media::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            media::tool_status,
            media::choose_ffmpeg,
            media::choose_video,
            media::choose_subtitle,
            media::choose_output,
            media::probe,
            media::inspect_subtitle,
            media::render_preview,
            media::start_export,
            media::start_compression,
            media::cancel_export,
            media::reveal_output,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Subtitle Burner");
}
