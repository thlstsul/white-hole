use hotkey::{Code, Modifiers, hotkey};
use log::error;
use tauri::AppHandle;

use crate::browser::BrowserExt as _;

#[hotkey([(Modifiers::CONTROL, Code::KeyR), (Modifiers::empty(), Code::F5)])]
async fn reload(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.reload().await {
        error!("刷新失败：{e}");
    }
}

#[hotkey(Modifiers::ALT, Code::ArrowLeft)]
async fn back(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.back().await {
        error!("后退失败：{e}");
    }
}

#[hotkey(Modifiers::ALT, Code::ArrowRight)]
async fn forward(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.forward().await {
        error!("前进失败：{e}");
    }
}

#[hotkey(Modifiers::CONTROL, Code::KeyL)]
async fn focus(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.focus().await {
        error!("进入主视图失败：{e}");
    }
    if let Err(e) = browser.focus_changed().await {
        error!("焦点变化失败：{e}");
    }
}

#[hotkey(Modifiers::empty(), Code::Escape)]
async fn blur(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.blur().await {
        error!("退出主视图失败：{e}");
    }
    if let Err(e) = browser.focus_changed().await {
        error!("焦点变化失败：{e}");
    }
}

#[hotkey(Modifiers::CONTROL, Code::KeyW)]
async fn close_tab(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.close_tab().await {
        error!("关闭标签失败: {e}");
    }
    if let Err(e) = browser.focus_changed().await {
        error!("焦点变化失败：{e}");
    }
}

#[hotkey(Modifiers::CONTROL, Code::Tab)]
async fn next_tab(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.next_tab().await {
        error!("切换标签失败：{e}");
    }
    if let Err(e) = browser.focus_changed().await {
        error!("焦点变化失败：{e}");
    }
}

#[hotkey(Modifiers::CONTROL | Modifiers::SHIFT, Code::Tab)]
async fn near_tab(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.near_tab().await {
        error!("切换标签失败：{e}");
    }
    if let Err(e) = browser.focus_changed().await {
        error!("焦点变化失败：{e}");
    }
}

#[hotkey(Modifiers::empty(), Code::F11)]
async fn fullscreen(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.fullscreen().await {
        error!("全屏失败: {e}");
    }
}

/// ？使用惯用快捷键会触发 devtools 快捷键，所以只能自定义快捷键
#[hotkey(Modifiers::CONTROL, Code::KeyD)]
async fn devtools(app_handle: AppHandle) {
    let browser = app_handle.browser();
    browser.devtools().await;
}

#[hotkey(Modifiers::CONTROL, Code::KeyI)]
async fn incognito(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.incognito().await {
        error!("切换标签失败：{e}");
    }
    focus(app_handle).await;
}

#[hotkey(Modifiers::CONTROL, Code::KeyP)]
async fn print(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.print().await {
        error!("打印失败：{e}");
    }
}
