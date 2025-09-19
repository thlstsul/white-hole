use log::error;
use tauri::async_runtime;
use tauri_plugin_updater::UpdaterExt as _;

pub fn update(app: tauri::AppHandle) {
    async_runtime::spawn(async move {
        let Ok(updater) = app.updater().inspect_err(|e| error!("创建更新器失败：{e}"))
        else {
            return;
        };

        let Ok(Some(_update)) = updater
            .check()
            .await
            .inspect_err(|e| error!("检查更新失败：{e}"))
        else {
            return;
        };

        // TODO: 提交到下载器
    });
}
