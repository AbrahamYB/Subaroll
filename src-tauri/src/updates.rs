use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::{Updater, UpdaterExt};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    installed_version: String,
    update: Option<UpdateInfo>,
}

fn fresh_updater(app: &AppHandle) -> Result<Updater, String> {
    app.updater_builder()
        .header("Cache-Control", "no-cache, max-age=0")
        .and_then(|builder| builder.build())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<UpdateCheck, String> {
    let updater = fresh_updater(&app)?;
    let update = updater.check().await.map_err(|error| error.to_string())?;
    Ok(UpdateCheck {
        installed_version: app.package_info().version.to_string(),
        update: update.map(|update| UpdateInfo {
            version: update.version,
            notes: update.body,
        }),
    })
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let updater = fresh_updater(&app)?;
    if let Some(update) = updater.check().await.map_err(|error| error.to_string())? {
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|error| error.to_string())?;
        app.restart();
    } else {
        Err("No update is available.".to_string())
    }
}
