mod media;
mod updates;

use media::AppState;
use tauri::Manager;

#[cfg(target_os = "macos")]
fn place_main_window(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let default_size = tauri::LogicalSize::new(880.0, 792.0);
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        window.set_size(default_size)?;
        return window.center();
    };
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    let mut size = default_size.to_physical::<u32>(scale);
    let margin = (24.0 * scale).round() as u32; // 12 logical points at each edge.
    size.height = size.height.min(area.size.height.saturating_sub(margin).max(1));
    let center = tauri::PhysicalPosition::new(
        area.position.x + (area.size.width as i32 - size.width as i32) / 2,
        area.position.y + (area.size.height as i32 - size.height as i32) / 2,
    );
    window.set_size(size)?;
    window.set_position(center)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::default())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unmaximize();
                #[cfg(target_os = "macos")]
                let _ = place_main_window(&window);
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = window.set_size(tauri::LogicalSize::new(880.0, 764.0));
                    let _ = window.center();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            media::tool_status,
            media::burn_quality_profile,
            media::choose_ffmpeg,
            media::choose_video,
            media::choose_subtitle,
            media::choose_output,
            media::find_subtitle,
            media::discover_subtitles,
            media::extract_subtitle,
            media::probe,
            media::inspect_subtitle,
            media::render_preview,
            media::start_export,
            media::start_compression,
            media::plan_compression,
            media::cancel_export,
            media::reveal_output,
            updates::check_for_update,
            updates::install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Subaroll")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                media::cancel_running_export(&app.state::<AppState>());
            }
        });
}
