use ::hotkey::HotkeyManagerExt as _;
use ::log::error;
use command::*;
use tauri::{
    AppHandle, DeviceEvent, DeviceId, Manager, Runtime, Webview, Window, WindowEvent,
    async_runtime, plugin::TauriPlugin,
};
use tauri_plugin_log::{Target, TargetKind, TimezoneStrategy};
use tauri_plugin_window_state::{AppHandleExt as _, StateFlags};
use time::macros::format_description;

use crate::{
    browser::{Browser, BrowserExt as _},
    error::SetupError,
    user_agent::setup_user_agent,
};

mod browser;
mod command;
mod database;
mod error;
mod hotkey;
mod icon;
mod log;
mod macros;
mod page;
mod public_suffix;
mod state;
mod tab;
mod task;
mod update;
mod url;
mod user_agent;

pub trait IsMainView {
    const TITLE_HEIGHT: f64 = 40.;
    const MAINVIEW_LABEL: &str = "main-view";
    fn is_main(&self) -> bool;
}

impl IsMainView for Webview {
    fn is_main(&self) -> bool {
        self.label() == Self::MAINVIEW_LABEL
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), SetupError> {
    setup_user_agent();

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(setup_log());

    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_single_instance::init(single_instance_init))
            .plugin(tauri_plugin_window_state::Builder::new().build())
            .plugin(::hotkey::init());
    }

    builder
        .setup(|app| {
            Browser::setup(app)?;
            update::update(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            focus,
            blur,
            get_state,
            search,
            open_tab,
            back,
            forward,
            go,
            reload,
            incognito,
            query_navigation_log,
            update_star,
            push_history_state,
            replace_history_state,
            pop_history_state,
            hash_changed,
            icon_changed,
            minimize,
            maximize,
            unmaximize,
            close,
            start_dragging,
            fullscreen_changed,
            leave_picture_in_picture,
        ])
        .on_window_event(on_window_event)
        .on_device_event(on_device_event)
        .run(tauri::generate_context!())?;

    Ok(())
}

fn single_instance_init(app: &AppHandle, args: Vec<String>, _cwd: String) {
    let Some(window) = app.get_window("main") else {
        return;
    };

    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    if args.len() < 2 {
        // 命令不带参数
        return;
    }

    async_runtime::spawn({
        let url = args[1].clone();
        async move {
            use crate::browser::BrowserExt as _;

            let browser = window.browser();
            let url = url::parse_keyword(None, &url).await.expect("非法链接");
            browser
                .open_tab_by_url(&url, true)
                .await
                .expect("打开链接失败");
        }
    });
}

fn on_window_event(window: &Window, event: &WindowEvent) {
    async_runtime::spawn({
        let window = window.clone();
        let event = event.clone();

        async move {
            if let WindowEvent::Destroyed = event
                && let Err(e) = window.app_handle().save_window_state(StateFlags::all())
            {
                error!("保存窗口状态失败：{e}");
            } else if let WindowEvent::Resized(_) = event {
                let browser = window.browser();
                if let Err(e) = browser.resize().await {
                    error!("重置浏览器大小失败：{e}");
                }
            } else if let WindowEvent::Focused(true) = event {
                // Webview::set_focus 后，会触发 WindowEvent::Focused 事件；所以 focus_changed 做了防抖
                let browser = window.browser();
                if let Ok(true) = browser
                    .focus_changed()
                    .await
                    .inspect_err(|e| error!("聚焦变更失败：{e}"))
                {
                    // 窗口重新聚焦时，清空残留已按下按键
                    let hotkey = window.hotkey();
                    hotkey.clear_pressed();
                }
            }
        }
    });
}

fn on_device_event<ID: DeviceId>(app: &AppHandle, _id: ID, event: DeviceEvent) {
    if let DeviceEvent::Key { pysical_key, state } = event {
        let hotkey = app.hotkey();
        hotkey.handle_key_event(pysical_key, state);
    }
}

fn setup_log<R: Runtime>() -> TauriPlugin<R> {
    use ::log::LevelFilter;

    let time_format =
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]");

    let builder = if cfg!(debug_assertions) {
        use colored::Colorize as _;
        use fern::colors::{Color, ColoredLevelConfig};

        let level_colors = ColoredLevelConfig::new()
            .error(Color::Red)
            .warn(Color::Yellow)
            .info(Color::Green)
            .debug(Color::Blue)
            .trace(Color::Magenta);

        tauri_plugin_log::Builder::new()
            .level(LevelFilter::Debug)
            .targets([
                Target::new(TargetKind::Stdout),
                Target::new(TargetKind::LogDir { file_name: None }),
                Target::new(TargetKind::Webview),
            ])
            .format(move |out, message, record| {
                // 获取当前时间并格式化为字符串
                let now = TimezoneStrategy::UseLocal.get_now();
                let now = now.format(time_format).unwrap_or_default().dimmed();

                // 创建带颜色的日志级别显示
                let level_colored = level_colors.color(record.level());

                let location = if let (Some(file), Some(line)) = (record.file(), record.line()) {
                    format!("{}:{}", file, line).cyan()
                } else {
                    "".cyan()
                };

                // 输出带颜色的日志信息
                out.finish(format_args!(
                    "{} [{}] {} - {}",
                    now,
                    level_colored,
                    location,
                    message.to_string().white()
                ));
            })
    } else {
        tauri_plugin_log::Builder::new()
            .level(LevelFilter::Error)
            .targets([
                Target::new(TargetKind::Stdout),
                Target::new(TargetKind::LogDir { file_name: None }),
                Target::new(TargetKind::Webview),
            ])
            .format(move |out, message, record| {
                // 获取当前时间并格式化为字符串
                let now = TimezoneStrategy::UseLocal.get_now();
                let now = now.format(time_format).unwrap_or_default();

                let location = if let (Some(file), Some(line)) = (record.file(), record.line()) {
                    format!("{}:{}", file, line)
                } else {
                    String::new()
                };

                out.finish(format_args!(
                    "{} [{}] {} - {}",
                    now,
                    record.level(),
                    location,
                    message
                ));
            })
    };

    builder.build()
}
