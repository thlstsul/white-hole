use std::{collections::HashMap, path::Path};

use log::error;
use tauri::{
    AppHandle, Manager, Wry,
    menu::{Menu, MenuEvent, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::broadcast::error::RecvError;

use downloader::{DownloadEvent, DownloadManager, DownloadStatus};

use crate::download::download_dir;

/// 托盘菜单中单个任务行的点击动作
#[derive(Clone, Copy, PartialEq)]
enum TrayAction {
    Pause,
    Resume,
    Open,
    Retry,
    Remove,
}

/// 托盘菜单中一行下载进度
#[derive(Clone)]
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
    /// 任务显示名（进度刷新 set_text 是整体替换，需用缓存的名称重建行文本，避免丢名）
    name: String,
}

/// 初始化托盘：创建托盘、启动事件驱动刷新下载进度。
///
/// 依赖 `download::setup` 已注册 `DownloadManager` 全局状态。
pub(crate) fn setup(app: &AppHandle) {
    if let Err(e) = build_tray(app) {
        error!("创建托盘失败：{e}");
    }

    let manager = app.state::<DownloadManager>().inner().clone();
    let app1 = app.clone();
    tauri::async_runtime::spawn(async move {
        tray_event_loop(app1, manager).await;
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

/// 事件驱动刷新托盘：替代原来的 1 秒轮询。
///
/// 同时监听两类下载事件流（由 downloader 新 API 提供）：
/// - `subscribe_events()`（广播通道）：任务生命周期事件（开始/暂停/恢复/完成/失败/取消），
///   携带任务 ID，用于判断是否需要重建菜单。
/// - `subscribe_all_progress()`（watch 通道）：任一任务的最新进度快照，用于实时刷新进度。
///
/// 菜单只在"结构"变化（任务增删、暂停/恢复切换、任务结束导致按钮增删）时才整体重建；
/// 下载过程中进度频繁变化，仅对已有菜单项调用 `set_text` 原地更新文本。
/// Windows 上 `tray.set_menu()` 会替换 HMENU，正在打开的托盘菜单会被立即关闭，
/// 而 `set_text` 走 `SetMenuItemInfoW` 原地修改，不会关闭已打开的菜单。
async fn tray_event_loop(app: AppHandle, manager: DownloadManager) {
    // 结构签名：任务 id + 操作按钮 + 静态名称（不含下载进度）
    let mut last_signature: Vec<String> = Vec::new();
    // 各任务的菜单项句柄，进度变化时用于原地 set_text
    let mut items: HashMap<String, TrayMenuItems> = HashMap::new();
    // 最近一次拉取的行数据，进度刷新时基于它重算 tooltip 百分比（tooltip 与行文本同步刷新）
    let mut last_rows: Vec<TrayRow> = Vec::new();

    // 初次刷新（应用启动时可能已有任务，不能干等事件流）
    refresh_tray(
        &app,
        &manager,
        &mut last_signature,
        &mut items,
        &mut last_rows,
    )
    .await;

    let mut events = manager.subscribe_events();
    let mut progress = manager.subscribe_all_progress();
    loop {
        tokio::select! {
            event = events.recv() => {
                match event {
                    // 生命周期事件：结构可能变化（任务增删/状态切换），重建菜单
                    Ok(event) => {
                        // TaskCompleted/Failed/Cancelled 已由 download::event_listener_loop
                        // 发系统通知，托盘只负责刷新菜单
                        let structural = matches!(
                            event,
                            DownloadEvent::TaskStarted { .. }
                                | DownloadEvent::TaskPaused { .. }
                                | DownloadEvent::TaskResumed { .. }
                                | DownloadEvent::TaskCompleted { .. }
                                | DownloadEvent::TaskFailed { .. }
                                | DownloadEvent::TaskCancelled { .. }
                        );
                        if structural {
                            refresh_tray(&app, &manager, &mut last_signature, &mut items, &mut last_rows).await;
                        }
                    }
                    // 广播丢帧：部分生命周期事件可能已被跳过，立即按权威状态重同步，
                    // 避免菜单卡在上一次结构（如已完成任务仍显示「▼ 下载中」）
                    Err(RecvError::Lagged(_)) => {
                        refresh_tray(&app, &manager, &mut last_signature, &mut items, &mut last_rows).await;
                    }
                    Err(_) => break,
                }
            }
            // 进度更新：实时刷新对应任务的进度文本，并同步刷新 tooltip 百分比
            _ = progress.changed() => {
                // 先 clone 快照再 await，避免跨 await 持有 watch::Ref（非 Send）
                let stats = progress.borrow().clone();
                if stats.task_id.is_empty() {
                    continue;
                }
                refresh_progress(
                    &app,
                    &manager,
                    &stats,
                    &mut items,
                    &mut last_rows,
                    &mut last_signature,
                )
                .await;
            }
        }
    }
}

/// 完整刷新托盘：批量拉取所有任务概览，重建或原地更新菜单，并刷新 tooltip。
async fn refresh_tray(
    app: &AppHandle,
    manager: &DownloadManager,
    last_signature: &mut Vec<String>,
    items: &mut HashMap<String, TrayMenuItems>,
    last_rows: &mut Vec<TrayRow>,
) {
    let rows = collect_rows(manager).await;
    // 缓存行数据：进度刷新（refresh_progress）时基于它重算 tooltip，无需再异步查询
    *last_rows = rows.clone();
    let signature = rows
        .iter()
        .map(|r| {
            let action = match r.action {
                Some(TrayAction::Pause) => "pause",
                Some(TrayAction::Resume) => "resume",
                Some(TrayAction::Open) => "open",
                Some(TrayAction::Retry) => "retry",
                Some(TrayAction::Remove) => "remove",
                None => "",
            };
            format!("{}|{action}|{}", r.task_id, r.name)
        })
        .collect::<Vec<_>>();

    if signature != *last_signature {
        match build_menu(app, &rows) {
            Ok((menu, new_items)) => {
                // 仅在构建成功后才提交签名：若构建失败，保留旧签名，
                // 下次结构变化时仍能重试重建，避免菜单卡在旧结构
                *last_signature = signature;
                *items = new_items;
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
        let _ = tray.set_tooltip(Some(&format_tooltip(&rows)));
    }
}

/// 轻量进度刷新：仅更新最近上报进度的那个任务的行文本与 tooltip，不重建菜单。
///
/// `stats` 来自 `subscribe_all_progress` 的快照（只含最近更新过进度的任务），
/// 配合 `items` 中缓存的菜单项句柄做原地 `set_text`，避免为每个进度事件重建菜单；
/// 同时基于 `last_rows` 缓存重算 tooltip，保证悬停百分比实时更新。
///
/// 快照不含任务状态，故以 `get_task_status` 回查的**权威状态**为准（`last_rows`
/// 缓存可能在事件流丢帧后过期）：任务已非「下载中」时不原地刷新行文本，而是触发
/// 一次完整 `refresh_tray` 按真实状态重建菜单——否则事件流丢失 TaskCompleted 后，
/// 完成时的最终快照（downloaded==total、speed==0）会把菜单永久刷成「▼ 下载中 · 等待」。
async fn refresh_progress(
    app: &AppHandle,
    manager: &DownloadManager,
    stats: &downloader::DownloadStats,
    items: &mut HashMap<String, TrayMenuItems>,
    last_rows: &mut Vec<TrayRow>,
    last_signature: &mut Vec<String>,
) {
    // 权威状态回查：进度快照不含状态，而 `last_rows` 缓存在事件流丢帧后会过期。
    // 以真实状态为准，与缓存行比对，任一不一致（任务已移除/已非下载中/缓存缺失）
    // 都触发一次完整 refresh_tray 按真实状态重建菜单——否则事件流丢失 TaskCompleted
    // 后，完成时的最终快照（downloaded==total、speed==0）会把菜单永久刷成「▼ 下载中 · 等待」。
    // 一致时缓存必然与权威状态同步（refresh_tray 每次都会刷新 last_rows），可安全走
    // 下方原地 set_text 快速路径；不一致分支以 watch 的最新值收敛，不会反复重建。
    let cached_active = last_rows
        .iter()
        .any(|r| r.task_id == stats.task_id && r.action == Some(TrayAction::Pause));
    let status = manager.get_task_status(&stats.task_id).await;
    let authoritative_active = status
        .as_ref()
        .map(|s| matches!(s, DownloadStatus::Pending | DownloadStatus::Downloading))
        .unwrap_or(false);
    if authoritative_active != cached_active {
        refresh_tray(app, manager, last_signature, items, last_rows).await;
        return;
    }
    // 仅当该任务当前在菜单里才更新；状态已切换的任务由上面的 refresh_tray 重建
    let Some(entry) = items.get_mut(&stats.task_id) else {
        return;
    };
    // 名称从缓存读取：set_text 是整体替换行文本，若用空名会把文件名刷掉
    let name = entry.name.clone();
    let line = format_task_line(&TrayRow {
        task_id: stats.task_id.clone(),
        name,
        downloaded: stats.downloaded,
        total: stats.total_size,
        speed: stats.speed,
        action: Some(TrayAction::Pause), // 仅下载中任务才有进度事件
    });
    let _ = entry.row.set_text(line);

    // 同步刷新 tooltip 百分比：把该任务的最新进度回填到缓存行后重算
    if let Some(row) = last_rows.iter_mut().find(|r| r.task_id == stats.task_id) {
        row.downloaded = stats.downloaded;
        row.total = stats.total_size;
        row.speed = stats.speed;
    }
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(&format_tooltip(last_rows)));
    }
}

async fn collect_rows(manager: &DownloadManager) -> Vec<TrayRow> {
    // 新 API：list_task_overviews 一次批量取回所有任务的概览，避免逐个任务多次异步查询
    let mut rows = Vec::new();
    for overview in manager.list_task_overviews().await {
        // 文件名尚未解析时回退为 URL 末段文件名，避免托盘菜单显示 UUID
        let name = match overview.filename {
            Some(name) => name,
            None => match downloader::extract_filename_from_url(&overview.url) {
                Some(name) => name,
                None => "下载中…".to_string(),
            },
        };
        let action = match overview.status {
            DownloadStatus::Pending | DownloadStatus::Downloading => Some(TrayAction::Pause),
            DownloadStatus::Paused => Some(TrayAction::Resume),
            DownloadStatus::Completed => Some(TrayAction::Open),
            // 失败的任务点击重试（Failed 状态可直接再次 start_task 续传）；
            // 取消的任务点击即移除，避免终态任务永久残留托盘菜单
            DownloadStatus::Failed(_) => Some(TrayAction::Retry),
            DownloadStatus::Cancelled => Some(TrayAction::Remove),
        };
        rows.push(TrayRow {
            task_id: overview.task_id,
            name,
            downloaded: overview.stats.downloaded,
            total: overview.stats.total_size,
            speed: overview.stats.speed,
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
            // 失败点击重试、取消点击移除任务
            let (item_id, enabled) = match row.action {
                Some(TrayAction::Pause) => (format!("pause-{}", row.task_id), true),
                Some(TrayAction::Resume) => (format!("resume-{}", row.task_id), true),
                Some(TrayAction::Open) => (format!("open-{}", row.task_id), true),
                Some(TrayAction::Retry) => (format!("retry-{}", row.task_id), true),
                Some(TrayAction::Remove) => (format!("remove-{}", row.task_id), true),
                None => (format!("task-{}", row.task_id), false),
            };
            let row_item =
                MenuItem::with_id(app, item_id, format_task_line(row), enabled, None::<&str>)?;
            menu.append(&row_item)?;
            items.insert(
                row.task_id.clone(),
                TrayMenuItems {
                    row: row_item,
                    name: row.name.clone(),
                },
            );
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
    // 行首图标表示任务状态：▼下载中 ⏸已暂停 ✓已完成 ↻失败(可重试) ✗已取消
    let icon = match row.action {
        Some(TrayAction::Pause) => "▼",
        Some(TrayAction::Resume) => "⏸",
        Some(TrayAction::Open) => "✓",
        Some(TrayAction::Retry) => "↻",
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
    if let Some(task_id) = id.strip_prefix("retry-") {
        // 失败任务重试：Failed 状态可直接再次 start_task（downloader 会从断点续传）
        let manager = app.state::<DownloadManager>().inner().clone();
        let task_id = task_id.to_string();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = manager.start_task(&task_id).await {
                error!("重试任务 {task_id} 失败：{e}");
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
