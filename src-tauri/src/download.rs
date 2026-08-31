use std::path::PathBuf;

use log::{error, info};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;

use downloader::{DownloadConfig, DownloadError, DownloadEvent, DownloadManager};

/// 初始化自建下载器：注册全局状态、启动后台监听任务终止事件。
///
/// 托盘（进度展示、任务操作入口）由 `crate::tray::setup` 独立初始化。
pub(crate) fn setup(app: &AppHandle) {
    let manager = DownloadManager::new();
    app.manage(manager.clone());

    let app1 = app.clone();
    tauri::async_runtime::spawn(async move {
        event_listener_loop(app1, manager).await;
    });
}

/// 将下载任务交给自建下载器（由 tab_service::on_download 调用）
///
/// 返回任务 ID，供调用方订阅完成事件（如自动更新）。
/// `cookies` 为已格式化好的 `name=value; ...` 字符串，用于携带登录态访问资源。
pub(crate) async fn start_download(
    app: &AppHandle,
    manager: DownloadManager,
    url: String,
    cookies: Option<String>,
) -> Result<String, String> {
    let dir = download_dir(app);
    let config = DownloadConfig {
        cookies,
        ..Default::default()
    };
    let task_id = manager.add_task(url.clone(), dir.clone(), config).await;
    info!("接管下载任务 {task_id}：{url}，保存目录 {}", dir.display());
    if let Err(e) = manager.start_task(&task_id).await {
        // 启动失败：移除刚注册的任务，避免孤儿任务残留在托盘/事件流中
        let _ = manager.remove_task(&task_id).await;
        return Err(format!("启动下载 {url} 失败：{e}"));
    }
    Ok(task_id)
}

/// 计算默认下载目录：优先系统下载目录，回退到应用数据目录。
///
/// 供下载启动与托盘「打开文件」共用。
pub(crate) fn download_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .download_dir()
        .or_else(|_| app.path().app_data_dir().map(|dir| dir.join("downloads")))
        .unwrap_or_else(|_| std::env::temp_dir().join("white-hole-downloads"))
}

/// 监听下载事件流（开始/完成/失败/取消），发送系统通知
///
/// Windows 下通过 `send_notification` 使用应用 AUMID + 自定义 appLogo 图标（见函数注释）；
/// 所有下载通知统一在此处理：开始（TaskStarted）、终止（完成/失败/取消），
/// 暂停/恢复为生命周期中间态，不打扰用户。
async fn event_listener_loop(app: AppHandle, manager: DownloadManager) {
    let mut rx = manager.subscribe_events();
    loop {
        match rx.recv().await {
            Ok(event) => {
                let (title, body) = match event {
                    DownloadEvent::TaskStarted { task_id } => {
                        let Some(url) = manager.get_task_url(&task_id).await else {
                            continue;
                        };
                        ("开始下载".to_string(), url)
                    }
                    DownloadEvent::TaskPaused { .. } | DownloadEvent::TaskResumed { .. } => {
                        continue;
                    }
                    DownloadEvent::TaskCompleted { task_id } => {
                        let name = manager
                            .get_task_filename(&task_id)
                            .await
                            .unwrap_or(task_id.clone());
                        let size = manager
                            .get_task_stats(&task_id)
                            .await
                            .map(|s| s.total_size)
                            .unwrap_or(0);
                        let dir = download_dir(&app);
                        let body = if size > 0 {
                            format!("{name}\n{} · 已保存到 {}", format_size(size), dir.display())
                        } else {
                            format!("{name}\n已保存到 {}", dir.display())
                        };
                        ("下载完成".to_string(), body)
                    }
                    DownloadEvent::TaskFailed { task_id, error } => {
                        let name = manager.get_task_filename(&task_id).await.unwrap_or(task_id);
                        let reason = friendly_error(&error);
                        (format!("下载失败 · {reason}"), name)
                    }
                    DownloadEvent::TaskCancelled { task_id } => {
                        let name = manager.get_task_filename(&task_id).await.unwrap_or(task_id);
                        ("下载已取消".to_string(), name)
                    }
                };
                send_notification(&app, &title, &body);
            }
            Err(RecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
}

/// 将字节数格式化为用户可读的大小（B / KB / MB / GB）
fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes / KB)
    } else {
        format!("{bytes:.0} B")
    }
}

/// 将下载错误转换为对用户友好的简短文案，避免把底层技术细节直接展示给用户
fn friendly_error(err: &DownloadError) -> String {
    match err {
        DownloadError::RequestTimeout | DownloadError::ReadTimeout => {
            "网络超时，请重试".to_string()
        }
        DownloadError::HttpError(e) => match e.status() {
            Some(status) => format!("服务器返回错误（HTTP {status}）"),
            None => "网络请求失败".to_string(),
        },
        DownloadError::IoError(_) => "本地写入失败，请检查磁盘空间".to_string(),
        DownloadError::JsonError(_) | DownloadError::InvalidStateFile => {
            "下载状态文件异常".to_string()
        }
        DownloadError::TaskNotFound => "下载任务不存在".to_string(),
        DownloadError::TokioError(_) => "下载内部错误".to_string(),
        DownloadError::Cancelled | DownloadError::Other(_) => "下载失败".to_string(),
    }
}

/// 发送系统通知。
///
/// Windows 下不使用 tauri-plugin-notification：该插件在开发模式下不设置 `app_id`，
/// notify-rust 会回退到 PowerShell 的 AUMID，导致通知显示 PowerShell 图标/名称。
/// 这里直接调用 tauri-winrt-notification，显式指定应用 AUMID（配合 lib.rs 中的
/// SetCurrentProcessExplicitAppUserModelID），使通知以应用身份显示；
/// 不设置 appLogoOverride 图标（避免左侧出现大图标），并显式 short 时长让通知自动关闭。
/// 其他平台仍走插件（macOS/Linux 插件工作正常）。
fn send_notification(app: &AppHandle, title: &str, body: &str) {
    #[cfg(windows)]
    {
        use tauri_winrt_notification::{Duration, Toast};
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

        // Toast 基于 WinRT，show() 前需保证当前线程已初始化 COM。
        // S_OK(0) 表示本次由我们完成初始化，结束后应 CoUninitialize；
        // S_FALSE(1) 表示线程此前已初始化过（复用即可，不重复 Uninitialize）。
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let initialized = hr.is_ok() && hr.0 == 0;
        if !hr.is_ok() {
            error!("初始化 COM 失败：{hr}");
        }

        let app_id = app.config().identifier.clone();
        if let Err(e) = Toast::new(&app_id)
            .duration(Duration::Short)
            .title(title)
            .text1(body)
            .show()
        {
            error!("发送下载通知失败：{e}");
        }

        if initialized {
            unsafe { CoUninitialize() };
        }
    }
    #[cfg(not(windows))]
    {
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            error!("发送下载通知失败：{e}");
        }
    }
}
