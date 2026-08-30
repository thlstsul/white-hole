use std::{collections::HashMap, path::Path, time::Duration};

use log::error;
use tauri::{
    AppHandle, Manager, Wry,
    menu::{Menu, MenuEvent, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
};

use downloader::{DownloadManager, DownloadStatus};

use crate::download::download_dir;

/// 托盘菜单中单个任务行的点击动作
#[derive(Clone, Copy, PartialEq)]
enum TrayAction {
    Pause,
    Resume,
    Open,
    Remove,
}

/// 托盘菜单中一行下载进度
struct TrayRow {
    task_id: String,
    name: String,
    downloaded: u64,
    total: u64,
    speed: f64,
    action: Option<TrayAction>,
}

/// 某个任务的进度行菜单项句柄，用于原地更新文本（避免重建整个菜单导致打开的托盘菜单被关闭）
struct TrayMenuItems {
    row: MenuItem<Wry>,
}

/// 初始化托盘：创建托盘、启动后台刷新任务进度。
///
/// 依赖 `download::setup` 已注册 `DownloadManager` 全局状态。
pub(crate) fn setup(app: &AppHandle) {
    if let Err(e) = build_tray(app) {
        error!("创建托盘失败：{e}");
    }

    let manager = app.state::<DownloadManager>().inner().clone();
    let app1 = app.clone();
    tauri::async_runtime::spawn(async move {
        tray_refresh_loop(app1, manager).await;
    });
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let Some(icon) = app.default_window_icon().cloned() else {
        error!("未找到默认窗口图标，跳过托盘创建");
        return Ok(());
    };
    let (menu, _) = build_menu(app, &[])?;
    let tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("white-hole")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(on_tray_icon_event)
        .on_menu_event(on_menu_event)
        .build(app)?;
    // 保持托盘图标存活（不随函数结束被 drop）
    app.manage(tray);
    Ok(())
}

/// 周期刷新托盘：汇总所有下载任务进度，更新 tooltip 与菜单。
///
/// 菜单只在"结构"变化（任务增删、暂停/恢复切换、任务结束导致按钮增删）时才整体重建；
/// 下载过程中进度每秒变化，仅对已有菜单项调用 `set_text` 原地更新文本。
/// Windows 上 `tray.set_menu()` 会替换 HMENU，正在打开的托盘菜单会被立即关闭，
/// 而 `set_text` 走 `SetMenuItemInfoW` 原地修改，不会关闭已打开的菜单。
async fn tray_refresh_loop(app: AppHandle, manager: DownloadManager) {
    // 结构签名：任务 id + 操作按钮 + 静态名称（不含下载进度）
    let mut last_signature: Vec<String> = Vec::new();
    // 各任务的菜单项句柄，进度变化时用于原地 set_text
    let mut items: HashMap<String, TrayMenuItems> = HashMap::new();
    loop {
        let rows = collect_rows(&manager).await;
        let tooltip = format_tooltip(&rows);

        let signature = rows
            .iter()
            .map(|r| {
                let action = match r.action {
                    Some(TrayAction::Pause) => "pause",
                    Some(TrayAction::Resume) => "resume",
                    Some(TrayAction::Open) => "open",
                    Some(TrayAction::Remove) => "remove",
                    None => "",
                };
                format!("{}|{action}|{}", r.task_id, r.name)
            })
            .collect::<Vec<_>>();
        let structural_change = signature != last_signature;
        if structural_change {
            last_signature = signature;
        }

        if structural_change {
            match build_menu(&app, &rows) {
                Ok((menu, new_items)) => {
                    items = new_items;
                    if let Some(tray) = app.tray_by_id("main") {
                        let _ = tray.set_menu(Some(menu));
                    }
                }
                Err(e) => error!("构建托盘菜单失败：{e}"),
            }
        } else {
            // 结构未变：仅原地更新各任务的进度文本，不替换菜单
            for row in &rows {
                if let Some(entry) = items.get(&row.task_id) {
                    let _ = entry.row.set_text(format_task_line(row));
                }
            }
        }

        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_tooltip(Some(&tooltip));
        }

        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}

async fn collect_rows(manager: &DownloadManager) -> Vec<TrayRow> {
    let mut rows = Vec::new();
    for task_id in manager.list_tasks().await {
        // 文件名尚未解析时回退为 URL 末段文件名，避免托盘菜单显示 UUID
        let name = match manager.get_task_filename(&task_id).await {
            Some(name) => name,
            None => match manager.get_task_url(&task_id).await {
                Some(url) => downloader::extract_filename_from_url(&url)
                    .unwrap_or_else(|| "下载中…".to_string()),
                None => "下载中…".to_string(),
            },
        };
        let (downloaded, total, speed) = match manager.get_task_stats(&task_id).await {
            Some(stats) => (stats.downloaded, stats.total_size, stats.speed),
            None => (0, 0, 0.0),
        };
        let action = match manager.get_task_status(&task_id).await {
            Some(DownloadStatus::Pending) | Some(DownloadStatus::Downloading) => {
                Some(TrayAction::Pause)
            }
            Some(DownloadStatus::Paused) => Some(TrayAction::Resume),
            Some(DownloadStatus::Completed) => Some(TrayAction::Open),
            // 失败/取消的任务点击即移除，避免终态任务永久残留托盘菜单
            Some(DownloadStatus::Failed(_)) | Some(DownloadStatus::Cancelled) => {
                Some(TrayAction::Remove)
            }
            _ => None,
        };
        rows.push(TrayRow {
            task_id,
            name,
            downloaded,
            total,
            speed,
            action,
        });
    }
    rows
}

fn build_menu(
    app: &AppHandle,
    rows: &[TrayRow],
) -> tauri::Result<(Menu<Wry>, HashMap<String, TrayMenuItems>)> {
    let menu = Menu::new(app)?;
    let mut items = HashMap::new();
    if rows.is_empty() {
        menu.append(&MenuItem::with_id(
            app,
            "no-task",
            "暂无下载任务",
            false,
            None::<&str>,
        )?)?;
    } else {
        for row in rows {
            // 任务行本身可点击：下载中点击暂停、暂停中点击恢复、已完成点击打开文件、
            // 失败/取消点击移除任务
            let (item_id, enabled) = match row.action {
                Some(TrayAction::Pause) => (format!("pause-{}", row.task_id), true),
                Some(TrayAction::Resume) => (format!("resume-{}", row.task_id), true),
                Some(TrayAction::Open) => (format!("open-{}", row.task_id), true),
                Some(TrayAction::Remove) => (format!("remove-{}", row.task_id), true),
                None => (format!("task-{}", row.task_id), false),
            };
            let row_item =
                MenuItem::with_id(app, item_id, format_task_line(row), enabled, None::<&str>)?;
            menu.append(&row_item)?;
            items.insert(row.task_id.clone(), TrayMenuItems { row: row_item });
            // 已完成任务行点击为「打开文件」，再追加一个独立的「移除」入口，
            // 保证终态任务（完成/失败/取消）都能从托盘清除
            if row.action == Some(TrayAction::Open) {
                let remove_item = MenuItem::with_id(
                    app,
                    format!("remove-{}", row.task_id),
                    "🗑 移除",
                    true,
                    None::<&str>,
                )?;
                menu.append(&remove_item)?;
            }
        }
    }
    Ok((menu, items))
}

fn format_task_line(row: &TrayRow) -> String {
    let percent = if row.total > 0 {
        format!("{:.1}%", row.downloaded as f64 * 100.0 / row.total as f64)
    } else {
        "…".to_string()
    };
    let name = if row.name.chars().count() > 24 {
        let mut name: String = row.name.chars().take(24).collect();
        name.push('…');
        name
    } else {
        row.name.clone()
    };
    // 行首图标表示任务状态：▼下载中 ⏸已暂停 ✓已完成 ✗已取消/失败
    let icon = match row.action {
        Some(TrayAction::Pause) => "▼",
        Some(TrayAction::Resume) => "⏸",
        Some(TrayAction::Open) => "✓",
        Some(TrayAction::Remove) => "✗",
        None => "✗",
    };
    let speed = match row.action {
        Some(TrayAction::Pause) => format!("  {}", format_speed(row.speed)),
        _ => String::new(),
    };
    format!("{icon} {name}  {percent}{speed}")
}

fn format_speed(speed: f64) -> String {
    if speed <= 0.0 {
        return "等待".to_string();
    }
    if speed >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", speed / 1024.0 / 1024.0)
    } else if speed >= 1024.0 {
        format!("{:.1} KB/s", speed / 1024.0)
    } else {
        format!("{speed:.0} B/s")
    }
}

fn format_tooltip(rows: &[TrayRow]) -> String {
    // 仅统计进行中任务（Pending/Downloading，对应 action 为 Pause）；
    // 已暂停/已完成/已取消/失败的任务不计入「下载中 N 个任务」。
    let active: Vec<_> = rows
        .iter()
        .filter(|r| r.action == Some(TrayAction::Pause))
        .collect();
    if active.is_empty() {
        return "white-hole".to_string();
    }
    let total: u64 = active.iter().map(|r| r.total).sum();
    let done: u64 = active.iter().map(|r| r.downloaded).sum();
    let percent = if total > 0 {
        done as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    format!(
        "white-hole · 下载中 {} 个任务 · {percent:.0}%",
        active.len()
    )
}

fn on_tray_icon_event(tray: &TrayIcon<Wry>, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        show_main_window(tray.app_handle());
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    if let Some(task_id) = id.strip_prefix("pause-") {
        let manager = app.state::<DownloadManager>().inner().clone();
        let task_id = task_id.to_string();
        tauri::async_runtime::spawn(async move {
            manager.pause_task(&task_id).await;
        });
        return;
    }
    if let Some(task_id) = id.strip_prefix("resume-") {
        let manager = app.state::<DownloadManager>().inner().clone();
        let task_id = task_id.to_string();
        tauri::async_runtime::spawn(async move {
            manager.resume_task(&task_id).await;
        });
        return;
    }
    if let Some(task_id) = id.strip_prefix("open-") {
        let manager = app.state::<DownloadManager>().inner().clone();
        let task_id = task_id.to_string();
        let dir = download_dir(app);
        tauri::async_runtime::spawn(async move {
            let filename = manager.get_task_filename(&task_id).await;
            if let Some(name) = filename {
                let path = dir.join(name);
                if let Err(e) = open_path(&path) {
                    error!("打开下载文件失败：{e}");
                }
            }
        });
        return;
    }
    if let Some(task_id) = id.strip_prefix("remove-") {
        let manager = app.state::<DownloadManager>().inner().clone();
        let task_id = task_id.to_string();
        tauri::async_runtime::spawn(async move {
            if !manager.remove_task(&task_id).await {
                error!("移除任务 {task_id} 失败或任务不存在");
            }
        });
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 用系统默认程序打开文件（Windows 用 explorer，macOS/Linux 用 open/xdg-open）
fn open_path(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer").arg(path).spawn()?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "当前平台暂不支持打开文件",
        ))
    }
}
