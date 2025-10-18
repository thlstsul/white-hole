use hotkey::{Builder, Code, Hotkey, Modifiers, hotkey};
use log::error;
use tauri::{AppHandle, Wry, async_runtime, plugin::TauriPlugin};

use crate::browser::BrowserExt as _;

pub fn setup() -> TauriPlugin<Wry> {
    Builder::new()
        .register(Hotkey::new(Modifiers::ALT, Code::ArrowLeft), back)
        .register(Hotkey::new(Modifiers::ALT, Code::ArrowRight), forward)
        .register(Hotkey::new(Modifiers::CONTROL, Code::KeyT), focus)
        .register(Hotkey::new(Modifiers::CONTROL, Code::KeyL), focus)
        .register(Hotkey::new(Modifiers::empty(), Code::Escape), blur)
        .register(Hotkey::new(Modifiers::CONTROL, Code::KeyW), close_tab)
        .register(Hotkey::new(Modifiers::CONTROL, Code::Tab), next_tab)
        .register(
            Hotkey::new(Modifiers::CONTROL | Modifiers::SHIFT, Code::Tab),
            near_tab,
        )
        .register(Hotkey::new(Modifiers::empty(), Code::F11), fullscreen)
        .build()
}

#[hotkey]
async fn back(app_handle: AppHandle) {
    let browser = app_handle.browser();
    browser.back().await;
}

#[hotkey]
async fn forward(app_handle: AppHandle) {
    let browser = app_handle.browser();
    browser.forward().await;
}

#[hotkey]
async fn focus(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.focus().await {
        error!("浏览器焦点失败：{e}");
    }
    if let Err(e) = browser.focus_changed().await {
        error!("浏览器焦点变化失败：{e}");
    }
}

#[hotkey]
async fn blur(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.blur().await {
        error!("浏览器焦点失败：{e}");
    }
    if let Err(e) = browser.focus_changed().await {
        error!("浏览器焦点变化失败：{e}");
    }
}

#[hotkey]
async fn close_tab(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.close_tab().await {
        error!("关闭标签失败: {e}");
    }
    if let Err(e) = browser.focus_changed().await {
        error!("浏览器焦点变化失败：{e}");
    }
}

#[hotkey]
async fn next_tab(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.next_tab().await {
        error!("浏览器切换标签失败：{e}");
    }
    if let Err(e) = browser.focus_changed().await {
        error!("浏览器焦点变化失败：{e}");
    }
}

#[hotkey]
async fn near_tab(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.near_tab().await {
        error!("浏览器切换标签失败：{e}");
    }
    if let Err(e) = browser.focus_changed().await {
        error!("浏览器焦点变化失败：{e}");
    }
}

#[hotkey]
async fn fullscreen(app_handle: AppHandle) {
    let browser = app_handle.browser();
    if let Err(e) = browser.fullscreen().await {
        error!("全屏失败: {e}");
    }
}
