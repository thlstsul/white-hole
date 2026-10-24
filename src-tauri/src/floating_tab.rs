//! 浮动 Tab 模块：在主窗口右下角叠加的小型预览 webview，
//! 不参与常规 TabMap 管理，独立持有 webview 引用与状态。
//!
//! 内聚边界：本模块拥有浮动 Tab 的数据结构、状态锁、位置布局与事件回调；
//! Browser 仅负责编排（何时打开/关闭/提升），不直接触碰浮动状态内部。

use log::error;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager as _, Theme, Url, Webview, WebviewBuilder,
    WebviewUrl, Window, Wry,
    async_runtime::{self, Mutex},
    webview::{NewWindowResponse, PageLoadEvent, PageLoadPayload},
};
use uuid::Uuid;

use crate::{
    IsMainView as _,
    browser::BrowserExt as _,
    error::FrameworkError,
    tab::{TabId, bg_color},
    tab_service::on_download,
    user_agent::get_user_agent,
};

/// 单个浮动 Tab 的运行时状态
pub(crate) struct FloatingTab {
    /// webview label（Uuid），用于 close 时定位
    pub label: TabId,
    /// 当前加载的 URL（promote 时用于创建常规 Tab）
    pub url: String,
    /// webview 引用（close / reparent / resize 操作）
    pub webview: Webview,
}

/// 浮动 Tab 状态（最多 1 个）：所有读写收敛在同一把锁后，
/// 消除 open/close/promote 并发时的竞态窗口。
#[derive(Default)]
pub(crate) struct FloatingTabState(Mutex<Option<FloatingTab>>);

impl FloatingTabState {
    /// 存入新浮动 Tab，返回旧的（如有）；调用方负责关闭旧 webview
    pub(crate) async fn replace(&self, floating: FloatingTab) -> Option<FloatingTab> {
        self.0.lock().await.replace(floating)
    }

    /// 原子取出当前浮动 Tab（如有）
    pub(crate) async fn take(&self) -> Option<FloatingTab> {
        self.0.lock().await.take()
    }

    /// 原子取出：锁内校验并转换 URL，校验失败时浮动 Tab 仍存活，
    /// 成功时返回 (转换产物, 浮动 Tab)
    pub(crate) async fn take_validated<T, E>(
        &self,
        validate: impl FnOnce(&str) -> Result<T, E>,
    ) -> Result<Option<(T, FloatingTab)>, E> {
        let mut lock = self.0.lock().await;
        match lock.as_ref() {
            Some(f) => {
                let value = validate(&f.url)?;
                Ok(lock.take().map(|f| (value, f)))
            }
            None => Ok(None),
        }
    }

    /// 页面加载完成时更新当前 URL（由浮动 Tab 专用回调调用）
    pub(crate) async fn update_url(&self, label: &str, url: Option<String>) {
        let mut lock = self.0.lock().await;
        if let Some(ref mut f) = *lock
            && f.label == label
            && let Some(url) = url
        {
            f.url = url;
        }
    }

    /// 将现有浮动 Tab 重新置顶（常规 Tab 创建/切换后可能覆盖它）
    pub(crate) async fn raise(&self, window: &Window) {
        if let Some(ref floating) = *self.0.lock().await {
            let _ = floating.webview.reparent(window);
        }
    }

    /// 浮动 Tab 跟随窗口缩放
    pub(crate) async fn resize(&self, window_size: LogicalSize<f64>) {
        let mut lock = self.0.lock().await;
        if let Some(ref mut f) = *lock {
            let (pos, size) = layout(window_size);
            if let Err(e) = f.webview.set_size(size) {
                error!("浮动 Tab set_size 失败：{e}");
            }
            if let Err(e) = f.webview.set_position(pos) {
                error!("浮动 Tab set_position 失败：{e}");
            }
        }
    }
}

/// 浮动 Tab 位置和尺寸计算（纯函数，复用于 resize / open）
pub(crate) fn layout(window_size: LogicalSize<f64>) -> (LogicalPosition<f64>, LogicalSize<f64>) {
    let window_width = window_size.width;
    let window_height = window_size.height;
    let content_h = (window_height - Webview::TITLE_HEIGHT).max(0.0);
    let float_w = window_width * 0.3;
    let float_h = content_h * 0.3;
    let margin = window_width * 0.02;
    let x = (window_width - float_w - margin).max(0.0);
    let y = (window_height - float_h - margin).max(Webview::TITLE_HEIGHT);
    (
        LogicalPosition::new(x, y),
        LogicalSize::new(float_w, float_h),
    )
}

/// 创建浮动 Tab 的 webview 构建器：注入暗色模式与浮动控制栏 JS，
/// 挂接浮动 Tab 专用事件回调。返回 (label, builder)。
pub(crate) fn webview_builder(
    window: &Window,
    url: &Url,
    incognito: bool,
) -> Result<(String, WebviewBuilder<Wry>), FrameworkError> {
    let label = format!("floating-{}", Uuid::now_v7());
    let is_dark = matches!(window.theme()?, Theme::Dark);
    let app_handle = window.app_handle().clone();
    let builder = WebviewBuilder::new(&label, WebviewUrl::External(url.clone()))
        .initialization_script(include_str!("../js/darkreader.js"))
        .initialization_script(include_str!("../js/floating_tab.js"))
        .user_agent(&get_user_agent())
        .incognito(incognito)
        .background_color(bg_color(is_dark))
        .devtools(true)
        .zoom_hotkeys_enabled(true)
        .focused(false) // 不抢焦点：FT-3 要求原 Tab 保持不变
        .on_new_window({
            let app_handle = app_handle.clone();
            move |url, _| on_floating_new_window(&app_handle, url)
        })
        .on_page_load(on_floating_page_load)
        .on_download(on_download);
    Ok((label, builder))
}

/// 浮动 Tab 内的 on_new_window：**始终**替换当前浮动 Tab（不区分 Ctrl 状态）。
/// 设计意图（见 SPEC FT-17）：浮动 Tab 内的 Ctrl+点击与普通点击行为一致——
/// 都是替换当前浮动 Tab 为新页面，保持"浮动 Tab 内不产生常规 Tab"的隔离性。
fn on_floating_new_window(app_handle: &AppHandle, url: Url) -> NewWindowResponse<Wry> {
    async_runtime::spawn({
        let app_handle = app_handle.clone();

        async move {
            let browser = app_handle.browser();
            // 替换：关闭旧浮动 → 打开新浮动
            browser
                .open_floating_tab(&url)
                .await
                .inspect_err(|e| error!("替换浮动链接{url}失败：{e}"))
        }
    });

    NewWindowResponse::Deny
}

/// 浮动 Tab 专用：页面加载状态回调（FT-14）
fn on_floating_page_load(webview: Webview, payload: PageLoadPayload) {
    let event = payload.event();
    async_runtime::spawn(async move {
        let label = webview.label();
        let loading = match event {
            PageLoadEvent::Started => true,
            PageLoadEvent::Finished => false,
        };
        // 页面加载完成时预读 URL（锁外调用，避免阻塞其他浮动 Tab 操作）
        let current_url = if !loading {
            webview.url().ok().map(|u| u.to_string())
        } else {
            None
        };
        let browser = webview.browser();
        browser.floating.update_url(label, current_url).await;
    });
}
