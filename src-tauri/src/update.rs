use tauri_plugin_updater::{Result, UpdaterExt as _};

pub async fn update(app: tauri::AppHandle) -> Result<()> {
    if let Some(_update) = app.updater()?.check().await? {
        // TODO: 提交到下载器
    }

    Ok(())
}
