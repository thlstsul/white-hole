use std::ops::Deref;

use log::{error, info};
use scc::HashMap;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager as _, Theme, Webview, WebviewUrl, Window, Wry,
    async_runtime::{self, RwLock},
    webview::{DownloadEvent, NewWindowResponse, PageLoadPayload},
    window::Color,
};
use tauri_plugin_notification::NotificationExt;
use url::Url;
use uuid::Uuid;

use crate::{
    IsMainView as _,
    browser::{BrowserExt, bg_color},
    darkreader::{DARKREADER_DISABLE_SCRIPT, DARKREADER_ENABLE_SCRIPT},
    error::FrameworkError,
    state::BrowserState,
    user_agent::get_user_agent,
};

const BLANK_URL: &str = "about:blank";

pub type TabId = String;

#[derive(Debug, Clone)]
struct HistoryEntry {
    id: i64,
    url: String,
    /// Navigation API 上报的条目身份（会话内唯一稳定）；降级路径下为 None
    key: Option<String>,
}

/// Navigation API 快照中的一条历史条目（key 作为条目身份）
#[derive(Debug, Clone, serde::Deserialize)]
pub struct HistorySnapshotEntry {
    pub key: String,
    pub url: String,
}

pub struct Tab {
    webview: Webview,
    title: String,
    icon_url: String,
    loading: bool,
    /// 等待中的同文档导航（pushState/popstate）或 bfcache 恢复：
    /// 不触发页面加载事件，需靠 Navigation API 快照确认导航完成并清理 loading
    nav_pending: bool,
    /// 加载期间到达但被跳过落库的标题（loading 守卫防止标题错位）：
    /// 待快照对账后用权威 URL 补写，避免同文档导航无 PageLoad Finished 事件而丢失
    pending_title: Option<String>,
    incognito: bool,
    darkreader: bool,
    index: isize,
    /// 当前加载是否为重定向（reload / 302 / meta refresh）：
    /// 浏览器对重定向是替换当前条目而非新增，sync 时据此选择语义
    redirecting: bool,
    history: Vec<HistoryEntry>,
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
        let is_dark = matches!(window.theme()?, Theme::Dark);
        let builder =
            tauri::webview::WebviewBuilder::new(&label, WebviewUrl::External(url.clone()))
                .initialization_script(include_str!("../js/darkreader.js"))
                .initialization_script(include_str!("../js/webview_init.js"))
                .initialization_script(include_str!("../js/copy_hook.js"))
                .initialization_script_for_all_frames(include_str!("../js/all_frames_init.js"))
                .user_agent(&get_user_agent())
                .incognito(incognito)
                .background_color(bg_color(is_dark))
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
            title: String::new(),
            icon_url: String::new(),
            loading: true,
            nav_pending: false,
            pending_title: None,
            incognito,
            darkreader: true,
            redirecting: false,
            history: Vec::new(),
            index: -1,
        })
    }

    pub fn index(&self, id: i64) -> Option<usize> {
        self.history
            .iter()
            .enumerate()
            .find_map(|(i, item)| if item.id == id { Some(i) } else { None })
    }

    pub fn insert_history(&mut self, id: i64, url: String, length: usize) {
        if id <= 0 {
            return;
        }

        if self.index < 0 || self.history.len() + 1 == length {
            self.history.push(HistoryEntry { id, url, key: None });
            self.index = (self.history.len() - 1) as isize;
            return;
        }

        let i = self.index as usize;
        if i < self.history.len() && id == self.history[i].id {
            self.history[i].url = url;
            return;
        }

        if length > 0 && self.history.len() > length {
            let truncate_to = length - 1;
            self.history.truncate(truncate_to);
            self.index = (length - 2) as isize;
        }

        self.history.push(HistoryEntry { id, url, key: None });
        self.index += 1;

        info!(
            "insert history, index: {}, history: {:?}, 实际历史长度: {}",
            self.index, self.history, length
        );
    }

    pub fn replace_history(&mut self, id: i64, url: String, length: usize) {
        if id <= 0 {
            return;
        }

        if length > 0 && self.history.len() > length {
            self.history.truncate(length);
            let max_index = (length - 1) as isize;
            if self.index > max_index {
                self.index = max_index;
            }
        }

        if self.index < 0 || self.history.len() + 1 == length {
            self.history.push(HistoryEntry { id, url, key: None });
            self.index = (self.history.len() - 1) as isize;
        } else {
            let i = self.index as usize;
            if i < self.history.len() {
                self.history[i] = HistoryEntry { id, url, key: None };
            }
        }

        info!(
            "replace history, index: {}, history: {:?}, 实际历史长度: {}",
            self.index, self.history, length
        );
    }

    pub fn can_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.index < self.history.len() as isize - 1
    }

    /// 当前条目 URL：优先取快照对账后的镜像（SPA pushState/replaceState 下
    /// webview 原生 URL 更新滞后，镜像才是权威值）；空镜像回退原生 URL
    pub fn current_url(&self) -> Result<String, tauri::Error> {
        let url = match self.history.get(self.index.max(0) as usize) {
            Some(entry) => entry.url.clone(),
            None => self.url()?.to_string(),
        };
        Ok(if url == BLANK_URL { String::new() } else { url })
    }

    pub fn back(&mut self) -> bool {
        if !self.can_back() {
            return false;
        }

        // 优先 Navigation API（权威导航），无则降级 history.back()
        if let Err(e) = self
            .webview
            .eval("(window.navigation ? navigation.back() : history.back())")
        {
            error!("{}后退失败{e}", self.label());
            false
        } else {
            // index 不再预更新，等待 currententrychange / popstate 回传后对账校准
            true
        }
    }

    pub fn forward(&mut self) -> bool {
        if !self.can_forward() {
            return false;
        }

        // 优先 Navigation API（权威导航），无则降级 history.forward()
        if let Err(e) = self
            .webview
            .eval("(window.navigation ? navigation.forward() : history.forward())")
        {
            error!("{}前进失败{e}", self.label());
            false
        } else {
            // index 不再预更新，等待 currententrychange / popstate 回传后对账校准
            true
        }
    }

    pub fn go(&mut self, index: usize) -> bool {
        let index = index as isize;
        if self.index == index {
            return false;
        }

        // 优先按 key 精确跳转（Navigation API），key 失效或缺失时退化为相对 delta，
        // 彻底摆脱"用镜像 index 算 delta"的漂移反馈环
        let script = match self.history.get(index as usize).and_then(|e| e.key.clone()) {
            Some(key) => {
                let key = serde_json::to_string(&key).unwrap_or_default();
                format!(
                    "navigation.traverseTo({key}).catch(function(){{ history.go({}) }})",
                    index - self.index
                )
            }
            None => format!("history.go({})", index - self.index),
        };
        if let Err(e) = self.webview.eval(script) {
            error!("{}跳转失败{e}", self.label());
            false
        } else {
            // index 不再预更新，等待 webview 事件回传后对账校准
            true
        }
    }

    pub fn sync_by_url(&mut self, url: &str, length: usize) -> bool {
        // 若 webview 回传的历史长度更短，截断后端多余历史
        if length > 0 && self.history.len() > length {
            let truncate_to = length;
            self.history.truncate(truncate_to);
            let max_index = (truncate_to.saturating_sub(1)) as isize;
            if self.index > max_index {
                self.index = max_index;
            }
        }

        // 查找 URL 是否已存在于历史栈：精确匹配优先，其次容忍尾部斜杠差异；
        // 有多个匹配时选择距离当前 index 最近的位置（最小移动量），
        // 避免 position() 固定选第一个匹配而在 URL 重复时定位错误
        let cur = self.index.max(0) as usize;
        let url_trimmed = url.trim_end_matches('/');
        let (mut best_exact, mut best_loose): (Option<usize>, Option<usize>) = (None, None);
        for (i, entry) in self.history.iter().enumerate() {
            if entry.url == url {
                if best_exact.is_none_or(|b| i.abs_diff(cur) < b.abs_diff(cur)) {
                    best_exact = Some(i);
                }
            } else if entry.url.trim_end_matches('/') == url_trimmed
                && best_loose.is_none_or(|b| i.abs_diff(cur) < b.abs_diff(cur))
            {
                best_loose = Some(i);
            }
        }

        if let Some(pos) = best_exact.or(best_loose) {
            if self.index != pos as isize {
                info!(
                    "sync_by_url: URL 匹配到位置 {}，index 从 {} 修正为 {}",
                    pos, self.index, pos
                );
                self.index = pos as isize;
            }
            return false; // 未插入新条目
        }

        // URL 不在历史栈中
        if self.redirecting && self.index >= 0 {
            // 重定向（reload/302/meta refresh）：浏览器替换当前条目而非新增，
            // 原地更新当前条目的 URL，id 置为占位等待回填
            let i = self.index as usize;
            if i < self.history.len() {
                self.history[i].url = url.to_string();
                self.history[i].id = -1;
                info!(
                    "sync_by_url: 重定向替换当前条目为 {url}，index: {}，history: {:?}",
                    self.index, self.history
                );
                return true; // 需要真实 id 回填
            }
        }

        // 正常导航：截断当前位置之后的 forward 历史，然后插入新条目
        // 防御 index 为 -1（空历史新 tab）导致的 usize 下溢
        let i = self.index.max(0) as usize;
        if i != self.history.len().saturating_sub(1) {
            self.history.truncate(i + 1);
        }
        self.history.push(HistoryEntry {
            id: -1,
            url: url.to_string(),
            key: None,
        });
        self.index = (self.history.len() - 1) as isize;

        info!(
            "sync_by_url: 插入未知 URL {}，index: {}, history: {:?}",
            url, self.index, self.history
        );
        true // 插入了新条目（id 占位符为 -1）
    }

    /// 以 Navigation API 上报的权威快照全量重建历史镜像。
    /// entries 按原生顺序排列；key 作为条目身份，同 key 且 URL 未变的条目
    /// 保留原 id；新 key 或同 key 但 URL 已变（replaceState 只改 URL 不改 key）
    /// 置占位 -1（等待 save_navigation_log 回填）。
    /// 返回需要回填 id 的 (位置, url) 列表。
    pub fn sync_snapshot(
        &mut self,
        index: usize,
        entries: Vec<HistorySnapshotEntry>,
    ) -> Vec<(usize, String)> {
        // key 作为条目身份：同 key 且 URL 未变的条目保留原 id，
        // 新 key 或 URL 变更（replaceState）置占位 -1 等待回填
        let id_by_key: std::collections::HashMap<&str, (i64, &str)> = self
            .history
            .iter()
            .filter_map(|entry| {
                entry
                    .key
                    .as_deref()
                    .map(|key| (key, (entry.id, entry.url.as_str())))
            })
            .collect();

        self.history = entries
            .into_iter()
            .map(|entry| HistoryEntry {
                id: match id_by_key.get(entry.key.as_str()) {
                    Some((id, url)) if *url == entry.url => *id,
                    _ => -1,
                },
                url: entry.url,
                key: Some(entry.key),
            })
            .collect();
        self.index = index as isize;
        self.redirecting = false;

        // 占位条目（新 key / replaceState 改 URL）等待 save_navigation_log 回填
        self.history
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.id <= 0)
            .map(|(i, entry)| (i, entry.url.clone()))
            .collect()
    }

    /// 将指定位置条目的占位 id（-1）回填为真实 id（快照对账后调用）
    pub fn backfill_history(&mut self, pos: usize, id: i64, url: String) {
        if id <= 0 {
            return;
        }
        if let Some(entry) = self.history.get_mut(pos) {
            entry.id = id;
            entry.url = url;
        }
    }

    pub fn reload(&self) {
        if let Err(e) = self.webview.reload() {
            error!("重载失败：{e}");
        }
    }

    pub fn set_darkreader(&mut self, enable: bool) -> Result<(), tauri::Error> {
        let result = if enable {
            self.eval(DARKREADER_ENABLE_SCRIPT)
        } else {
            self.eval(DARKREADER_DISABLE_SCRIPT)
        };

        if result.is_ok() {
            self.darkreader = enable;
        }

        result
    }
}

pub struct TabIndex(RwLock<TabId>);

impl TabIndex {
    pub fn new() -> Self {
        Self(RwLock::new(TabId::new()))
    }

    pub async fn get(&self) -> TabId {
        self.0.read().await.clone()
    }

    pub async fn set(&self, label: TabId) {
        *self.0.write().await = label;
    }

    pub async fn eq(&self, label: &str) -> bool {
        *self.0.read().await == label
    }

    pub async fn clear(&self) {
        self.0.write().await.clear();
    }
}

pub struct TabMap(HashMap<TabId, Tab>);

impl TabMap {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub async fn insert(&self, label: TabId, tab: Tab) {
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
    pub async fn any_open(&self, id: i64, incognito: bool) -> Option<(TabId, usize)> {
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
                if let Err(e) = tab.set_size(size) {
                    error!("设置webview大小失败：{e}");
                }
                true
            })
            .await;
    }

    pub async fn set_position(&self, position: LogicalPosition<f64>) {
        self.0
            .iter_async(|_, tab| {
                if let Err(e) = tab.set_position(position) {
                    error!("设置webview位置失败：{e}")
                }
                true
            })
            .await;
    }

    pub async fn set_title(&self, label: &str, title: String) {
        self.0.update_async(label, |_, tab| tab.title = title).await;
    }

    /// 记录加载期间被跳过落库的标题，待快照对账后用权威 URL 补写
    pub async fn set_pending_title(&self, label: &str, title: String) {
        self.0
            .update_async(label, |_, tab| tab.pending_title = Some(title))
            .await;
    }

    /// 读取并清除待补写的标题
    pub async fn take_pending_title(&self, label: &str) -> Option<String> {
        self.0
            .update_async(label, |_, tab| tab.pending_title.take())
            .await
            .flatten()
    }

    pub async fn set_icon(&self, label: &str, icon_url: String) {
        self.0
            .update_async(label, |_, tab| tab.icon_url = icon_url)
            .await;
    }

    pub async fn start_loading(&self, label: &str) {
        self.0
            .update_async(label, |_, tab| {
                tab.loading = true;
                // 避免污染 icon
                tab.icon_url.clear();
            })
            .await;
    }

    pub async fn set_loading(&self, label: &str, loading: bool) {
        self.0
            .update_async(label, |_, tab| tab.loading = loading)
            .await;
    }

    pub async fn is_loading(&self, label: &str) -> bool {
        self.0
            .read_async(label, |_, tab| tab.loading)
            .await
            .unwrap_or(false)
    }

    pub async fn set_nav_pending(&self, label: &str, pending: bool) {
        self.0
            .update_async(label, |_, tab| tab.nav_pending = pending)
            .await;
    }

    /// 读取并清除待定导航标记；true 表示存在未被页面加载事件确认的导航
    pub async fn take_nav_pending(&self, label: &str) -> bool {
        self.0
            .update_async(label, |_, tab| {
                let pending = tab.nav_pending;
                tab.nav_pending = false;
                pending
            })
            .await
            .unwrap_or(false)
    }

    pub async fn insert_history(&self, label: &str, id: i64, url: String, length: usize) {
        self.0
            .update_async(label, |_, tab| tab.insert_history(id, url, length))
            .await;
    }

    pub async fn replace_history(&self, label: &str, id: i64, url: String, length: usize) {
        self.0
            .update_async(label, |_, tab| tab.replace_history(id, url, length))
            .await;
    }

    pub async fn sync_by_url(&self, label: &str, url: String, length: usize) -> bool {
        self.0
            .update_async(label, |_, tab| tab.sync_by_url(&url, length))
            .await
            .unwrap_or(false)
    }

    pub async fn set_redirecting(&self, label: &str, redirecting: bool) {
        self.0
            .update_async(label, |_, tab| tab.redirecting = redirecting)
            .await;
    }

    /// 全量对账：以权威快照重建镜像，返回需要回填 id 的 (位置, url) 列表
    pub async fn sync_snapshot(
        &self,
        label: &str,
        index: usize,
        entries: Vec<HistorySnapshotEntry>,
    ) -> Vec<(usize, String)> {
        self.0
            .update_async(label, |_, tab| tab.sync_snapshot(index, entries))
            .await
            .unwrap_or_default()
    }

    /// 回填指定位置条目的 id（快照对账后）
    pub async fn backfill_history(&self, label: &str, pos: usize, id: i64, url: String) {
        self.0
            .update_async(label, |_, tab| tab.backfill_history(pos, id, url))
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
        self.0.read_async(label, |_, tab| tab.reload()).await;
    }

    pub async fn set_darkreader(&self, label: &str, enable: bool) -> Result<(), tauri::Error> {
        self.0
            .update_async(label, |_, tab| tab.set_darkreader(enable))
            .await
            .unwrap_or(Ok(()))
    }

    pub async fn set_background_color(
        &self,
        label: &str,
        color: Color,
    ) -> Result<(), tauri::Error> {
        self.0
            .update_async(label, |_, tab| tab.set_background_color(Some(color)))
            .await
            .unwrap_or(Ok(()))
    }

    pub async fn darkreader(&self, label: &str) -> Result<bool, tauri::Error> {
        self.0
            .update_async(label, |_, tab| {
                tab.set_darkreader(!tab.darkreader).map(|_| tab.darkreader)
            })
            .await
            .unwrap_or(Ok(true))
    }

    pub async fn devtools(&self, label: &str) {
        self.0
            .read_async(label, |_, tab| {
                if tab.is_devtools_open() {
                    tab.close_devtools();
                } else {
                    tab.open_devtools();
                }
            })
            .await;
    }

    pub async fn print(&self, label: &str) -> Result<(), FrameworkError> {
        self.0
            .read_async(label, |_, tab| tab.print())
            .await
            .unwrap_or(Err(tauri::Error::WebviewNotFound))?;

        Ok(())
    }

    pub async fn get_state(&self, label: &str) -> Result<BrowserState, FrameworkError> {
        let state = self
            .0
            .read_async(label, |_, tab| {
                Ok(BrowserState {
                    icon_url: tab.icon_url.clone(),
                    title: tab.title.clone(),
                    url: tab.current_url()?,
                    loading: tab.loading,
                    can_back: tab.can_back(),
                    can_forward: tab.can_forward(),
                    darkreader: tab.darkreader,
                    ..Default::default()
                })
            })
            .await
            .unwrap_or(Err(tauri::Error::WebviewNotFound))?;

        Ok(state)
    }

    pub async fn next(&self, label: &str) -> Option<TabId> {
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

    pub async fn near(&self, label: &str) -> Option<TabId> {
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
        let loading = match event {
            tauri::webview::PageLoadEvent::Started => true,
            tauri::webview::PageLoadEvent::Finished => false,
        };

        browser
            .on_page_load(label, loading)
            .await
            .inspect_err(|e| error!("{label}变更加载状态失败：{e}"))
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
