use ::log::error;
use command::*;
use tauri::{Manager, Webview, async_runtime};
use tauri_plugin_log::{Target, TargetKind, TimezoneStrategy};
use time::macros::format_description;

use crate::{browser::Browser, error::SetupError, user_agent::setup_user_agent};

mod browser;
mod command;
mod database;
mod error;
mod icon;
mod log;
mod macros;
mod page;
mod public_suffix;
mod shortcut;
mod state;
mod tab;
mod task;
mod update;
mod url;
mod user_agent;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), SetupError> {
    setup_user_agent();

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(setup_log().build());

    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, args, _| {
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
                        browser
                            .state_changed(None)
                            .await
                            .expect("浏览器状态同步失败");
                    }
                });
            }))
            .plugin(tauri_plugin_window_state::Builder::new().build())
            .plugin(shortcut::plugin()?);
    }

    builder
        .setup(|app| {
            Browser::setup(app)?;
            async_runtime::spawn({
                let app_handle = app.handle().clone();
                async move {
                    if let Err(e) = update::update(app_handle).await {
                        error!("检查更新失败: {}", e);
                    }
                }
            });
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
            icon_changed,
            minimize,
            maximize,
            unmaximize,
            close,
            start_dragging,
            fullscreen_changed,
        ])
        .on_window_event(|window, event| {
            async_runtime::spawn({
                let w = window.clone();
                let e = event.clone();

                async move {
                    let _ = Browser::on_window_event(&w, &e)
                        .await
                        .inspect_err(|e| error!("窗口事件处理失败：{e}"));
                }
            });
        })
        .run(tauri::generate_context!())?;
    Ok(())
}

fn setup_log() -> tauri_plugin_log::Builder {
    use ::log::LevelFilter;

    let time_format =
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]");

    if cfg!(debug_assertions) {
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
    }
}

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
