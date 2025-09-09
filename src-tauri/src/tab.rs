use std::ops::Deref;

use log::{error, info};
use scc::HashMap;
use tauri::{
    LogicalPosition, LogicalSize, Manager as _, Webview, WebviewUrl, Window,
    async_runtime::{self, RwLock},
    webview::{NewWindowResponse, PageLoadEvent},
};
use url::Url;
use uuid::Uuid;

use crate::{
    IsMainView as _,
    browser::{Browser, BrowserExt},
    error::{FrameworkError, StateError},
    state::BrowserState,
};

pub struct Tab {
    webview: Webview,
    label: String,
    title: String,
    icon_url: String,
    loading: bool,
    incognito: bool,
    index: i32,
    history_states: Vec<i64>,
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

        let label = Uuid::now_v7().to_string();

        let app_handle = window.app_handle().clone();

        let webview = window.add_child(
            tauri::webview::WebviewBuilder::new(&label, WebviewUrl::External(url.clone()))
                .initialization_script(include_str!("webview_init_script.js"))
                .incognito(incognito)
                .devtools(true)
                .zoom_hotkeys_enabled(true)
                .on_new_window(move |url, _| {
                    async_runtime::spawn({
                        let url = url.clone();
                        let app_handle = app_handle.clone();

                        async move {
                            if let Err(e) = Browser::on_new_window(&app_handle, &url).await {
                                error!("{e}");
                            }
                        }
                    });
                    NewWindowResponse::Deny
                })
                .on_document_title_changed(move |webview, title| {
                    async_runtime::spawn(async move {
                        if let Err(e) = on_document_title_changed(webview, title).await {
                            error!("{e}");
                        }
                    });
                })
                .on_page_load(|w, p| {
                    let event = p.event();
                    async_runtime::spawn(async move {
                        if let Err(e) = on_page_load(w, event).await {
                            error!("{e}");
                        }
                    });
                }),
            LogicalPosition::new(0., Webview::TITLE_HEIGHT),
            size,
        )?;

        Ok(Self {
            webview,
            label,
            title: String::new(),
            icon_url: String::new(),
            loading: true,
            incognito,
            index: -1,
            history_states: Vec::new(),
        })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    pub fn icon_url(&self) -> &str {
        &self.icon_url
    }

    pub fn set_icon_url(&mut self, icon_url: String) {
        self.icon_url = icon_url;
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub fn incognito(&self) -> bool {
        self.incognito
    }

    pub fn index(&self, id: i64) -> Option<usize> {
        self.history_states
            .iter()
            .enumerate()
            .find_map(|(i, item)| if *item == id { Some(i) } else { None })
    }

    pub fn insert_history(&mut self, id: i64) -> i32 {
        if self.index < 0 {
            self.history_states.push(id);
            self.index = (self.history_states.len() - 1) as i32;
            return self.index;
        }

        let i = self.index as usize;
        if id == self.history_states[i] {
            return self.index;
        }

        if i != self.history_states.len() - 1 {
            self.history_states.truncate(i + 1);
        }
        self.history_states.push(id);
        self.index += 1;
        self.index
    }

    pub fn replace_history(&mut self, id: i64) -> i32 {
        if self.index < 0 {
            self.history_states.push(id);
            self.index = (self.history_states.len() - 1) as i32;
        } else {
            self.history_states[self.index as usize] = id;
        }
        self.index
    }

    pub fn can_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.index < self.history_states.len() as i32 - 1
    }

    pub fn back(&mut self) {
        if !self.can_back() {
            return;
        }

        if self
            .webview
            .eval("history.back()")
            .inspect_err(|e| error!("{e}"))
            .is_ok()
        {
            self.index -= 1;
        }
    }

    pub fn forward(&mut self) {
        if !self.can_forward() {
            return;
        }

        if self
            .webview
            .eval("history.forward()")
            .inspect_err(|e| error!("{e}"))
            .is_ok()
        {
            self.index += 1;
        }
    }

    pub fn go(&mut self, index: usize) {
        let index = index as i32;
        if self.index != index
            && self
                .webview
                .eval(format!("history.go({})", index - self.index))
                .inspect_err(|e| error!("{e}"))
                .is_ok()
        {
            self.index = index;
        }
    }

    pub fn reload(&self) {
        let _ = self.webview.reload().inspect_err(|e| error!("{e}"));
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
            .scan_async(|l, tab| {
                if tab.incognito() {
                    labels.push(l.to_owned());
                }
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
                if tab.incognito() != incognito {
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

    pub async fn set_size(&self, size: LogicalSize<f64>) {
        self.0
            .scan_async(|_, tab| {
                let _ = tab.set_size(size);
            })
            .await;
    }

    pub async fn set_title(&self, label: &str, title: String) {
        self.0
            .update_async(label, |_, tab| tab.set_title(title))
            .await;
    }

    pub async fn set_icon(&self, label: &str, icon_url: String) {
        self.0
            .update_async(label, |_, tab| tab.set_icon_url(icon_url))
            .await;
    }

    pub async fn set_loading(&self, label: &str, loading: bool) {
        self.0
            .update_async(label, |_, tab| tab.set_loading(loading))
            .await;
    }

    pub async fn insert_history(&self, label: &str, id: i64) {
        self.0
            .update_async(label, |_, tab| tab.insert_history(id))
            .await;
    }

    pub async fn replace_history(&self, label: &str, id: i64) {
        self.0
            .update_async(label, |_, tab| tab.replace_history(id))
            .await;
    }

    pub async fn back(&self, label: &str) {
        self.0.update_async(label, |_, tab| tab.back()).await;
    }

    pub async fn forward(&self, label: &str) {
        self.0.update_async(label, |_, tab| tab.forward()).await;
    }

    pub async fn go(&self, label: &str, index: usize) {
        self.0.update_async(label, |_, tab| tab.go(index)).await;
    }

    pub async fn reload(&self, label: &str) {
        self.0.update_async(label, |_, tab| tab.reload()).await;
    }

    pub async fn get_state(&self, label: &str) -> Result<BrowserState, FrameworkError> {
        let state = self
            .0
            .read_async(label, |_, tab| {
                Ok(BrowserState {
                    icon_url: tab.icon_url().to_owned(),
                    title: tab.title().to_owned(),
                    url: tab.url()?.to_string(),
                    loading: tab.loading(),
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
        if self.0.len() < 2 {
            return None;
        }

        let mut rtn = None::<String>;
        let mut max = label.to_owned();
        self.0
            .scan_async(|l, _| {
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
            })
            .await;

        if rtn.is_none() && max != label {
            Some(max)
        } else {
            rtn
        }
    }

    pub async fn near(&self, label: &str) -> Option<String> {
        if self.0.len() < 2 {
            return None;
        }

        let mut rtn = None::<String>;
        self.0
            .scan_async(|l, _| {
                if l.as_str() > label {
                    if rtn.is_none() {
                        rtn = Some(l.to_owned());
                    } else if let Some(ref r) = rtn
                        && l < r
                    {
                        rtn = Some(l.to_owned());
                    }
                }
            })
            .await;

        if rtn.is_none() {
            self.next(label).await
        } else {
            rtn
        }
    }
}

async fn on_document_title_changed(webview: Webview, title: String) -> Result<(), StateError> {
    let label = webview.label();
    info!("{label} webview title changed: {title}");

    let browser = webview.browser();
    browser.change_tab_title(label, title).await;

    let state = browser.get_state(None).await?;
    if browser.is_current_tab(label).await {
        browser.state_changed(Some(state.clone())).await?;
    }
    browser.save_navigation_log(state.into()).await?;

    Ok(())
}

async fn on_page_load(webview: Webview, event: PageLoadEvent) -> Result<(), StateError> {
    info!("{} webview page load: {event:?}", webview.label());

    let browser = webview.browser();
    match event {
        tauri::webview::PageLoadEvent::Started => {
            browser
                .change_tab_loading_state(webview.label(), true)
                .await
        }
        tauri::webview::PageLoadEvent::Finished => {
            browser
                .change_tab_loading_state(webview.label(), false)
                .await
        }
    }
}
