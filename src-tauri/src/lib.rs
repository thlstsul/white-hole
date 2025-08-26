use std::sync::OnceLock;

use ::log::LevelFilter;
use browser::*;
use tauri::{App, Manager, State, Webview, Window, async_runtime, command};
use tauri_plugin_log::{Target, TargetKind};

use crate::error::{FrameworkError, StateError};

mod browser;
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
mod url;

pub const DB_NAME: &str = "white-hole.db";
pub static DB_URL: OnceLock<String> = OnceLock::new();

pub fn get_db_url(app: &App) -> Result<&String, FrameworkError> {
    let data_path = app.path().app_local_data_dir()?;
    let db_path = data_path.join(DB_NAME);
    Ok(DB_URL.get_or_init(|| format!("sqlite:{}", db_path.to_string_lossy())))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), FrameworkError> {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(if cfg!(debug_assertions) {
                    LevelFilter::Debug
                } else {
                    LevelFilter::Error
                })
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                    Target::new(TargetKind::Webview),
                ])
                .build(),
        );

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _| {
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

            let url = args[1].clone();
            async_runtime::spawn(async move {
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
            });
        }));

        builder = builder.plugin(tauri_plugin_window_state::Builder::new().build());
    }

    builder
        .setup(|app| {
            Browser::setup(app)?;
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
            start_dragging
        ])
        .on_window_event(|window, event| {
            let w = window.clone();
            let e = event.clone();
            async_runtime::spawn(async move {
                let _ = Browser::on_window_event(&w, &e).await;
            });
        })
        .run(tauri::generate_context!())?;
    Ok(())
}

#[command]
fn minimize(window: Window, mainview: Webview) {
    if !mainview.is_main() {
        return;
    }
    let _ = window.minimize();
}

#[command]
async fn maximize(browser: State<'_, Browser>, mainview: Webview) -> Result<(), StateError> {
    if !mainview.is_main() {
        return Ok(());
    }

    browser.maximize()?;
    browser.state_changed(None).await
}

#[command]
async fn unmaximize(browser: State<'_, Browser>, mainview: Webview) -> Result<(), StateError> {
    if !mainview.is_main() {
        return Ok(());
    }

    browser.unmaximize()?;
    browser.state_changed(None).await
}

#[command]
fn close(window: Window, mainview: Webview) {
    if !mainview.is_main() {
        return;
    }
    let _ = window.close();
}

#[command]
fn start_dragging(window: Window, mainview: Webview) {
    if !mainview.is_main() {
        return;
    }
    let _ = window.start_dragging();
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
