use std::ops::Deref;

use log::{error, info};
use scc::HashMap;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager as _, Webview, WebviewUrl, Window, Wry,
    async_runtime::{self, RwLock},
    webview::{DownloadEvent, NewWindowResponse, PageLoadPayload},
};
use tauri_plugin_notification::NotificationExt;
use url::Url;
use uuid::Uuid;

use crate::{
    IsMainView as _,
    browser::BrowserExt,
    darkreader::{DARKREADER_DISABLE_SCRIPT, DARKREADER_ENABLE_SCRIPT},
    error::FrameworkError,
    state::BrowserState,
    user_agent::get_user_agent,
};

const LOADING_TITLE: &str = "正在加载……";
const BLANK_URL: &str = "about:blank";

pub struct Tab {
    webview: Webview,
    title: String,
    icon_url: String,
    loading: bool,
    incognito: bool,
    index: isize,
    history: Vec<i64>,
}

impl Deref for Tab {
    type Target = Webview;

    fn deref(&self) -> &Self::Target {
        &self.webview
    }
}

impl Tab {
    pub fn new(window: &Window, url: &Url, incognito: bool) -> Result<Self, FrameworkError> {
        let mut size = window
            .inner_size()?
            .to_logical::<f64>(window.scale_factor()?);
        size.height -= Webview::TITLE_HEIGHT;
        let position = LogicalPosition::new(0., Webview::TITLE_HEIGHT);

        let label = Uuid::now_v7().to_string();
        let app_handle = window.app_handle().clone();
        let builder =
            tauri::webview::WebviewBuilder::new(&label, WebviewUrl::External(url.clone()))
                .initialization_script(include_str!("../js/darkreader.js"))
                .initialization_script(include_str!("../js/webview_init.js"))
                .initialization_script_for_all_frames(include_str!("../js/all_frames_init.js"))
                .initialization_script_for_all_frames(include_str!(
                    "../js/prevent_default_hotkey.js"
                ))
                .user_agent(&get_user_agent())
                .incognito(incognito)
                .devtools(true)
                .zoom_hotkeys_enabled(true)
                .focused(true)
                .on_new_window(move |url, _| on_new_window(&app_handle, url))
                .on_document_title_changed(on_document_title_changed)
                .on_page_load(on_page_load)
                .on_download(on_download);

        let webview = window.add_child(builder, position, size)?;

        Ok(Self {
            webview,
            title: LOADING_TITLE.to_string(),
            icon_url: String::new(),
            loading: true,
            incognito,
            history: Vec::new(),
            index: -1,
        })
    }

    pub fn index(&self, id: i64) -> Option<usize> {
        self.history
            .iter()
            .enumerate()
            .find_map(|(i, item)| if *item == id { Some(i) } else { None })
    }

    pub fn insert_history(&mut self, id: i64, length: i32) {
        if id <= 0 {
            return;
        }

        let length = length as usize;
        if self.history.len() > length {
            self.history.truncate(length);
        }

        if self.index < 0 || self.history.len() + 1 == length {
            self.history.push(id);
            self.index = (self.history.len() - 1) as isize;
            return;
        }

        let i = self.index as usize;
        if id == self.history[i] {
            return;
        }

        if self.history.len() == length {
            self.history[self.index as usize] = id;
        } else {
            self.history.push(id);
            self.index += 1;
        }

        info!(
            "insert history, index: {}, history_states: {:?}, 实际历史长度: {}",
            self.index, self.history, length
        );
    }

    pub fn replace_history(&mut self, id: i64, length: i32) {
        if id <= 0 {
            return;
        }

        if self.index < 0 {
            self.history.push(id);
            self.index = (self.history.len() - 1) as isize;
        } else {
            self.history[self.index as usize] = id;
        }
        info!(
            "replace history, index: {}, history_states: {:?}, 实际历史长度: {}",
            self.index, self.history, length
        );
    }

    pub fn can_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.index < self.history.len() as isize - 1
    }

    pub fn back(&mut self) -> bool {
        if !self.can_back() {
            return false;
        }

        if let Err(e) = self.webview.eval("history.back()") {
            error!("{}后退失败{e}", self.label());
            false
        } else {
            self.index -= 1;
            true
        }
    }

    pub fn forward(&mut self) -> bool {
        if !self.can_forward() {
            return false;
        }

        if let Err(e) = self.webview.eval("history.forward()") {
            error!("{}前进失败{e}", self.label());
            false
        } else {
            self.index += 1;
            true
        }
    }

    pub fn go(&mut self, index: usize) -> bool {
        let index = index as isize;
        if self.index == index {
            return false;
        }

        if let Err(e) = self
            .webview
            .eval(format!("history.go({})", index - self.index))
        {
            error!("{}跳转失败{e}", self.label());
            false
        } else {
            self.index = index;
            true
        }
    }

    pub fn reload(&self) {
        let _ = self
            .webview
            .reload()
            .inspect_err(|e| error!("重载失败：{e}"));
    }

    pub fn darkreader(&self, enable: bool) -> Result<(), tauri::Error> {
        if enable {
            self.eval(DARKREADER_ENABLE_SCRIPT)
        } else {
            self.eval(DARKREADER_DISABLE_SCRIPT)
        }
    }
}

pub struct TabIndex(RwLock<String>);

impl TabIndex {
    pub fn new() -> Self {
        Self(RwLock::new(String::new()))
    }

    pub async fn get(&self) -> String {
        self.0.read().await.clone()
    }

    pub async fn set(&self, label: String) {
        *self.0.write().await = label;
    }

    pub async fn eq(&self, label: &str) -> bool {
        *self.0.read().await == label
    }
}

pub struct TabMap(HashMap<String, Tab>);

impl TabMap {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub async fn insert(&self, label: String, tab: Tab) {
        self.0.upsert_async(label, tab).await;
    }

    pub async fn close(&self, label: &str) -> Result<(), FrameworkError> {
        let Some((_, tab)) = self.0.remove_async(label).await else {
            return Ok(());
        };

        tab.close()?;
        Ok(())
    }

    pub async fn close_incognito(&self) -> Result<(), FrameworkError> {
        let mut labels = Vec::new();
        self.0
            .iter_async(|l, tab| {
                if tab.incognito {
                    labels.push(l.to_owned());
                }
                true
            })
            .await;
        for label in labels {
            self.close(&label).await?;
        }
        Ok(())
    }

    /// return id 所在 (label, index)
    pub async fn any_open(&self, id: i64, incognito: bool) -> Option<(String, usize)> {
        let mut label = None;
        self.0
            .any_async(|l, tab| {
                if tab.incognito != incognito {
                    return false;
                }

                let Some(index) = tab.index(id) else {
                    return false;
                };

                label = Some((l.to_owned(), index));
                true
            })
            .await;
        label
    }

    pub async fn top(&self, label: &str, window: &Window) -> Result<(), FrameworkError> {
        self.0
            .read_async(label, |_, tab| tab.reparent(window))
            .await
            .unwrap_or(Err(tauri::Error::WebviewNotFound))?;
        Ok(())
    }

    pub async fn set_focus(&self, label: &str) -> Result<(), FrameworkError> {
        self.0
            .read_async(label, |_, tab| tab.set_focus())
            .await
            .unwrap_or(Err(tauri::Error::WebviewNotFound))?;
        Ok(())
    }

    pub async fn set_size(&self, size: LogicalSize<f64>) {
        self.0
            .iter_async(|_, tab| {
                let _ = tab
                    .set_size(size)
                    .inspect_err(|e| error!("设置webview大小失败：{e}"));
                true
            })
            .await;
    }

    pub async fn set_position(&self, position: LogicalPosition<f64>) {
        self.0
            .iter_async(|_, tab| {
                let _ = tab
                    .set_position(position)
                    .inspect_err(|e| error!("设置webview位置失败：{e}"));
                true
            })
            .await;
    }

    pub async fn set_title(&self, label: &str, title: String) {
        self.0.update_async(label, |_, tab| tab.title = title).await;
    }

    pub async fn set_icon(&self, label: &str, icon_url: String) {
        self.0
            .update_async(label, |_, tab| tab.icon_url = icon_url)
            .await;
    }

    pub async fn set_loading(&self, label: &str, loading: bool) {
        self.0
            .update_async(label, |_, tab| tab.loading = loading)
            .await;
    }

    pub async fn insert_history(&self, label: &str, id: i64, length: i32) {
        self.0
            .update_async(label, |_, tab| tab.insert_history(id, length))
            .await;
    }

    pub async fn replace_history(&self, label: &str, id: i64, length: i32) {
        self.0
            .update_async(label, |_, tab| tab.replace_history(id, length))
            .await;
    }

    pub async fn back(&self, label: &str) -> bool {
        self.0
            .update_async(label, |_, tab| tab.back())
            .await
            .unwrap_or(false)
    }

    pub async fn forward(&self, label: &str) -> bool {
        self.0
            .update_async(label, |_, tab| tab.forward())
            .await
            .unwrap_or(false)
    }

    pub async fn go(&self, label: &str, index: usize) -> bool {
        self.0
            .update_async(label, |_, tab| tab.go(index))
            .await
            .unwrap_or(false)
    }

    pub async fn reload(&self, label: &str) {
        self.0.update_async(label, |_, tab| tab.reload()).await;
    }

    pub async fn darkreader(&self, label: &str, enable: bool) -> Result<(), tauri::Error> {
        self.0
            .update_async(label, |_, tab| tab.darkreader(enable))
            .await
            .unwrap_or(Ok(()))
    }

    pub async fn get_state(&self, label: &str) -> Result<BrowserState, FrameworkError> {
        let state = self
            .0
            .read_async(label, |_, tab| {
                let mut url = tab.url()?.to_string();
                if url == BLANK_URL {
                    url.clear();
                }
                Ok(BrowserState {
                    icon_url: tab.icon_url.clone(),
                    title: tab.title.clone(),
                    url,
                    loading: tab.loading,
                    can_back: tab.can_back(),
                    can_forward: tab.can_forward(),
                    ..Default::default()
                })
            })
            .await
            .unwrap_or(Err(tauri::Error::WebviewNotFound))?;

        Ok(state)
    }

    pub async fn next(&self, label: &str) -> Option<String> {
        if self.0.is_empty() {
            return None;
        }

        let mut rtn = None::<String>;
        let mut max = label.to_owned();
        self.0
            .iter_async(|l, _| {
                if l.as_str() < label {
                    if rtn.is_none() {
                        rtn = Some(l.to_owned());
                    } else if let Some(ref r) = rtn
                        && l > r
                    {
                        rtn = Some(l.to_owned());
                    }
                }

                if l > &max {
                    max = l.to_owned();
                }
                true
            })
            .await;

        if rtn.is_none() && max != label {
            Some(max)
        } else {
            rtn
        }
    }

    pub async fn near(&self, label: &str) -> Option<String> {
        if self.0.is_empty() {
            return None;
        }

        let mut rtn = None::<String>;
        self.0
            .iter_async(|l, _| {
                if l.as_str() > label {
                    if rtn.is_none() {
                        rtn = Some(l.to_owned());
                    } else if let Some(ref r) = rtn
                        && l < r
                    {
                        rtn = Some(l.to_owned());
                    }
                }
                true
            })
            .await;

        if rtn.is_none() {
            self.next(label).await
        } else {
            rtn
        }
    }
}

fn on_new_window(app_handle: &AppHandle, url: Url) -> NewWindowResponse<Wry> {
    async_runtime::spawn({
        let app_handle = app_handle.clone();

        async move {
            let browser = app_handle.browser();
            browser.set_loading(false).await;
            browser
                .open_tab_by_url(&url, true)
                .await
                .inspect_err(|e| error!("打开链接{url}失败：{e}"))
        }
    });

    NewWindowResponse::Deny
}

fn on_document_title_changed(webview: Webview, title: String) {
    async_runtime::spawn(async move {
        let label = webview.label();
        info!("{label} webview title changed: {title}");

        let browser = webview.browser();
        browser
            .change_tab_title(label, title)
            .await
            .inspect_err(|e| error!("{label}变更标题失败：{e}"))
    });
}

fn on_page_load(webview: Webview, payload: PageLoadPayload) {
    let event = payload.event();
    async_runtime::spawn(async move {
        let label = webview.label();
        info!("{label} webview page load: {event:?}");

        let browser = webview.browser();
        match event {
            tauri::webview::PageLoadEvent::Started => browser
                .change_tab_loading_state(label, true)
                .await
                .inspect_err(|e| error!("{label}变更加载状态失败：{e}")),
            tauri::webview::PageLoadEvent::Finished => browser
                .change_tab_loading_state(label, false)
                .await
                .inspect_err(|e| error!("{label}变更加载状态失败：{e}")),
        }
    });
}

fn on_download(webview: Webview, event: DownloadEvent) -> bool {
    if let Err(e) = match event {
        DownloadEvent::Requested { url, .. } => {
            let notification = webview.notification();
            notification.builder().title("下载").body(url).show()
        }
        DownloadEvent::Finished { url, success, .. } => {
            let notification = webview.notification();
            if success {
                notification.builder().title("下载完成").body(url).show()
            } else {
                notification.builder().title("下载失败").body(url).show()
            }
        }
        _ => Ok(()),
    } {
        error!("下载事件处理失败：{e}");
    }
    // TODO 使用自建下载器
    true
}
