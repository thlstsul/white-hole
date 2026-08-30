use log::error;
use tauri::{AppHandle, Manager, async_runtime};
use tauri_plugin_updater::UpdaterExt as _;
use tokio::sync::broadcast::error::RecvError;

use crate::download;
use downloader::{DownloadEvent, DownloadManager, DownloadStatus};

pub fn update(app: AppHandle) {
    async_runtime::spawn(async move {
        let Ok(updater) = app.updater().inspect_err(|e| error!("创建更新器失败：{e}"))
        else {
            return;
        };

        let Ok(Some(update)) = updater
            .check()
            .await
            .inspect_err(|e| error!("检查更新失败：{e}"))
        else {
            return;
        };

        // 签名公钥来自 tauri 配置（plugins.updater.pubkey，base64 编码的 minisign 公钥文本）
        let Some(pubkey) = read_pubkey(&app) else {
            error!("未配置更新签名公钥");
            return;
        };

        // 先订阅事件，避免下载任务先于订阅完成导致事件丢失
        let manager = app.state::<DownloadManager>().inner().clone();
        let mut rx = manager.subscribe_events();

        // 提交到自建下载器，避免插件内置下载与下载器重复下载
        let url = update.download_url.to_string();
        let Ok(task_id) = download::start_download(&app, manager.clone(), url.clone(), None)
            .await
            .inspect_err(|e| error!("提交更新包到下载器失败：{e}"))
        else {
            return;
        };
        log::info!("更新包已提交到下载器：{url}（任务 {task_id}）");

        // 等待该任务完成
        loop {
            match rx.recv().await {
                Ok(DownloadEvent::TaskCompleted { task_id: id }) if id == task_id => break,
                Ok(DownloadEvent::TaskFailed { task_id: id, error }) if id == task_id => {
                    error!("更新包下载失败：{error}");
                    return;
                }
                Ok(DownloadEvent::TaskCancelled { task_id: id }) if id == task_id => {
                    error!("更新包下载被取消");
                    return;
                }
                Ok(_) => continue,
                // 广播丢帧：终止事件可能被跳过，以任务状态兜底判断是否已进入终态，
                // 避免错过 TaskCompleted 后永久阻塞等待
                Err(RecvError::Lagged(_)) => match manager.get_task_status(&task_id).await {
                    Some(DownloadStatus::Completed) => break,
                    Some(DownloadStatus::Failed(error)) => {
                        error!("更新包下载失败：{error}");
                        return;
                    }
                    Some(DownloadStatus::Cancelled) => {
                        error!("更新包下载被取消");
                        return;
                    }
                    _ => continue,
                },
                Err(_) => return,
            }
        }

        // 读取下载完成的文件字节
        let Some(filename) = manager.get_task_filename(&task_id).await else {
            error!("无法获取更新包文件名");
            return;
        };
        let path = download::download_dir(&app).join(&filename);
        let Ok(bytes) = std::fs::read(&path).inspect_err(|e| error!("读取更新包失败：{e}"))
        else {
            return;
        };

        // 校验签名，通过后才安装（防止下载源被篡改）
        if let Err(e) = verify_signature(&bytes, &update.signature, &pubkey) {
            error!("更新包签名校验失败：{e}");
            return;
        }

        if let Err(e) = update.install(bytes) {
            error!("安装更新失败：{e}");
        }
    });
}

/// 从 tauri 配置读取 updater 插件的签名公钥
fn read_pubkey(app: &AppHandle) -> Option<String> {
    app.config()
        .plugins
        .0
        .get("updater")?
        .get("pubkey")?
        .as_str()
        .map(str::to_string)
}

/// 复刻 tauri-plugin-updater 的 minisign 签名校验逻辑
fn verify_signature(data: &[u8], release_signature: &str, pubkey: &str) -> Result<(), String> {
    use base64::Engine as _;
    use minisign_verify::{PublicKey, Signature};

    let base64_to_string = |s: &str| -> Result<String, String> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(|e| format!("base64 解码失败：{e}"))?;
        String::from_utf8(bytes).map_err(|e| format!("UTF-8 解码失败：{e}"))
    };

    let public_key =
        PublicKey::decode(&base64_to_string(pubkey)?).map_err(|e| format!("解析公钥失败：{e}"))?;
    let signature = Signature::decode(&base64_to_string(release_signature)?)
        .map_err(|e| format!("解析签名失败：{e}"))?;

    public_key
        .verify(data, &signature, true)
        .map_err(|e| format!("签名校验失败：{e}"))
}
