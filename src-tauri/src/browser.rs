use crate::{
    IsMainView,
    darkreader::{self, delete_blacklist, save_blacklist},
    database::Database,
    error::*,
    icon::{get_cached_icon, get_icon_data_url},
    log::{NavigationLog, QueryLogResponse, get_id, get_url, query_log, save_log, update_log_star},
    page::PageToken,
    public_suffix::get_public_suffix_cached,
    state::{Boolean, BrowserState},
    tab::{HistorySnapshotEntry, Tab, TabId, TabIndex, TabMap},
    task,
    url::parse_keyword,
};
use log::error;
use tauri::{
    App, AppHandle, Emitter as _, LogicalPosition, Manager, State, Theme, Url, Webview,
    WebviewBuilder, WebviewUrl, Window, Wry,
    async_runtime::{self, Mutex},
    window::Color,
};
use tauri_plugin_window_state::{StateFlags, WindowExt};
use tokio::time::Instant;

const WIDTH: f64 = 800.;
const HEIGHT: f64 = 600.;
const FOCUS_LINK_TITLE: &str = "点击链接：";
const LOADING_TITLE: &str = "正在加载……";

/// 历史事件：所有历史写入统一经 FIFO 队列由单消费者按序应用，
/// 避免并发 IPC 命令乱序应用镜像。
pub enum HistoryEvent {
    Snapshot {
        index: usize,
        entries: Vec<HistorySnapshotEntry>,
    },
    Load {
        url: String,
        icon_url: String,
        length: usize,
    },
    /// 后端 on_page_load 完成时的历史校准
    LoadFinished {
        url: String,
        title: String,
        icon_url: String,
    },
}

/// 队列消息：标签 + 事件
type HistoryMessage = (TabId, HistoryEvent);

/// 历史事件消费者：严格按入队顺序应用
async fn consume_history(mut rx: async_runtime::Receiver<HistoryMessage>, app_handle: AppHandle) {
    while let Some((label, event)) = rx.recv().await {
        let browser = app_handle.browser();
        let _ = browser.apply_history_event(&label, event).await;
    }
}

pub(crate) fn bg_color(is_dark: bool) -> Color {
    if is_dark {
        Color(29, 35, 42, 255)
    } else {
        Color(255, 255, 255, 255)
    }
}

pub struct Browser {
    db: Database,
    window: Window,
    mainview: Webview,
    label: TabIndex,
    tabs: TabMap,
    is_focused: Boolean,
    incognito: Boolean,
    last_focus_changed: Mutex<Instant>,
    /// 历史事件 FIFO 队列（单消费者按序应用）
    history_queue: async_runtime::Sender<HistoryMessage>,
}

impl Browser {
    pub fn setup(app: &mut App) -> Result<(), SetupError> {
        async_runtime::block_on(async {
            let window = tauri::window::WindowBuilder::new(app, "main")
                .title("白洞")
                .inner_size(WIDTH, HEIGHT)
                .min_inner_size(WIDTH, HEIGHT)
                .decorations(false)
                .transparent(true)
                .focused(true)
                .build()?;

            window.restore_state(StateFlags::all())?;

            let mainview = window.add_child(
                Self::init_mainview(),
                LogicalPosition::new(0., 0.),
                window.inner_size()?,
            )?;

            let is_dark = matches!(window.theme()?, Theme::Dark);
            let bg = bg_color(is_dark);
            let _ = window.set_background_color(Some(bg));
            let _ = mainview.set_background_color(Some(bg));

            let db = Database::new(app).await?;

            // 历史事件单消费者：严格按入队顺序应用
            let (history_queue, history_rx) = async_runtime::channel::<HistoryMessage>(1024);
            let app_handle = app.handle().clone();
            async_runtime::spawn(consume_history(history_rx, app_handle));

            let state = Browser {
                db,
                window,
                mainview,
                label: TabIndex::new(),
                tabs: TabMap::new(),
                is_focused: Boolean::default(),
                incognito: Boolean::default(),
                last_focus_changed: Mutex::new(Instant::now()),
                history_queue,
            };
            app.manage(state);

            task::setup()?;

            Ok(())
        })
    }

    pub async fn resize(&self) -> Result<(), StateError> {
        let scale_factor = self.window.scale_factor()?;
        let mut web_size = self.window.inner_size()?.to_logical::<f64>(scale_factor);
        if !(self.label.get().await.is_empty()
            || web_size.height < HEIGHT
            || web_size.width < WIDTH)
        {
            // 无TAB或最小化后，不需要变更大小
            web_size.height -= Webview::TITLE_HEIGHT;
            self.tabs.set_size(web_size).await;
        }

        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn close_tab(&self) -> Result<(), TabError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        let label = self.label.get().await;
        self.tabs.close(&label).await?;
        self.label.clear().await;

        if let Some(near_label) = self.tabs.near(&label).await {
            self.switch_tab(&near_label).await?;
        }

        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn open_tab_by_url(&self, url: &Url, _active: bool) -> Result<(), TabError> {
        let pool = self.db.get().await;
        let incognito = self.incognito.get().await;
        self.is_focused.set(false).await;
        if let Some(id) = get_id(&pool, url.as_str()).await
            && let Some((label, index)) = self.tabs.any_open(id, incognito).await
        {
            self.tabs.go(&label, index).await;
            self.switch_tab(&label).await?;
            self.state_changed(None).await?;
        } else {
            let label = self.create_tab(url, true).await?;
            let mut state = self.get_state(None).await?;
            state.url = url.to_string();
            self.state_changed(Some(state.clone())).await?;
            let mut log: NavigationLog = state.into();
            log.title.clear();
            let id = self.save_navigation_log(log).await?;
            self.tabs
                .insert_history(&label, id, url.to_string(), 1)
                .await;
        }

        self.focus_changed().await?;
        Ok(())
    }

    pub async fn open_tab(&self, id: i64) -> Result<(), TabError> {
        let incognito = self.incognito.get().await;
        self.is_focused.set(false).await;
        if let Some((label, index)) = self.tabs.any_open(id, incognito).await {
            self.tabs.go(&label, index).await;
            self.switch_tab(&label).await?;
            self.state_changed(None).await?;
        } else if let Some(url) = get_url(self.db.get().await.as_ref(), id).await {
            let label = self.create_tab(&Url::parse(&url)?, true).await?;
            let mut state = self.get_state(None).await?;
            state.url = url.clone();
            self.state_changed(Some(state.clone())).await?;
            let mut log: NavigationLog = state.into();
            log.title.clear();
            let id = self.save_navigation_log(log).await?;
            self.tabs.insert_history(&label, id, url, 1).await;
        }

        self.focus_changed().await?;
        Ok(())
    }

    pub async fn next_tab(&self) -> Result<(), TabError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        let label = self.label.get().await;
        if let Some(next_label) = self.tabs.next(&label).await {
            self.switch_tab(&next_label).await?;

            self.state_changed(None).await?;
        }
        Ok(())
    }

    pub async fn near_tab(&self) -> Result<(), TabError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        let label = self.label.get().await;
        if let Some(near_label) = self.tabs.near(&label).await {
            self.switch_tab(&near_label).await?;

            self.state_changed(None).await?;
        }
        Ok(())
    }

    pub async fn is_current_tab(&self, label: &str) -> bool {
        self.label.eq(label).await
    }

    pub async fn change_tab_title(&self, label: &str, title: String) -> Result<(), StateError> {
        self.tabs.set_title(label, title).await;

        let mut state = self.get_state(Some(label)).await?;
        self.darkreader_auto_switch(label, &mut state).await;

        if self.is_current_tab(label).await {
            self.state_changed(Some(state.clone())).await?;
        }

        self.save_navigation_log(state.into()).await?;
        Ok(())
    }

    pub async fn content_loaded(
        &self,
        label: &str,
        url: String,
        length: i32,
        icon_url: String,
    ) -> Result<(), StateError> {
        self.tabs.set_icon(label, icon_url).await;

        let mut state = self.get_state(Some(label)).await?;
        self.darkreader_auto_switch(label, &mut state).await;

        if self.is_current_tab(label).await {
            self.state_changed(Some(state.clone())).await?;
        }

        let length = length as usize;
        let needs_id = self.tabs.sync_by_url(label, url.clone(), length).await;
        if needs_id {
            // 仅当 sync_by_url 插入了新条目（占位 id=-1）时才落库回填，
            // 避免 URL 命中旧条目时误覆盖旧条目的 id
            let id = self.save_navigation_log(state.into()).await?;
            self.tabs.replace_history(label, id, url, length).await;
        }

        Ok(())
    }

    pub async fn on_page_load(&self, label: &str, loading: bool) -> Result<(), StateError> {
        if loading {
            // 页面已在加载中又触发 Started = 重定向链（302/meta refresh/reload）
            let redirecting = self.tabs.is_loading(label).await;
            self.tabs.start_loading(label).await;
            self.tabs.set_redirecting(label, redirecting).await;
            return Ok(());
        }

        self.tabs.set_loading(label, loading).await;

        let state = self.get_state(Some(label)).await?;
        if self.is_current_tab(label).await {
            self.state_changed(Some(state.clone())).await?;
        }

        // 页面加载完成时，以实际 URL 校准历史栈（入队串行化，避免与 content_loaded 并发乱序）
        let url = state.url.clone();
        if !url.is_empty() && url != "about:blank" {
            self.enqueue_history(
                label,
                HistoryEvent::LoadFinished {
                    url,
                    title: state.title.clone(),
                    icon_url: state.icon_url.clone(),
                },
            )
            .await;
        }
        Ok(())
    }

    pub async fn set_loading(&self, loading: bool) {
        let label = self.label.get().await;
        if label.is_empty() {
            return;
        }

        self.tabs.set_loading(&label, loading).await;
    }

    /// Navigation API 权威快照对账：全量重建镜像，新 key 或 URL 变更（replaceState）
    /// 条目落库并回填 id，最后刷新当前标签页 UI 状态（back/forward 按钮）
    pub async fn sync_snapshot(
        &self,
        label: &str,
        index: usize,
        entries: Vec<HistorySnapshotEntry>,
    ) -> Result<(), StateError> {
        let needs_id = self.tabs.sync_snapshot(label, index, entries).await;
        // 快照不含 title/icon，当前条目（replaceState 改 URL）落库时从标签页取
        let state = self.tabs.get_state(label).await?;
        for (pos, url) in needs_id {
            let mut log = NavigationLog {
                url: url.clone(),
                ..Default::default()
            };
            if pos == index {
                log.title = state.title.clone();
                log.icon_url = state.icon_url.clone();
            }
            let id = self.save_navigation_log(log).await?;
            self.tabs.backfill_history(label, pos, id, url).await;
        }
        if self.is_current_tab(label).await {
            self.state_changed(Some(state)).await?;
        }
        Ok(())
    }

    /// 将历史事件入队（FIFO，由单消费者按序应用；队列满时等待背压）
    pub async fn enqueue_history(&self, label: impl Into<String>, event: HistoryEvent) {
        let _ = self.history_queue.send((label.into(), event)).await;
    }

    /// 队列消费者：按事件类型分发到具体处理逻辑
    async fn apply_history_event(
        &self,
        label: &str,
        event: HistoryEvent,
    ) -> Result<(), StateError> {
        match event {
            HistoryEvent::Snapshot { index, entries, .. } => {
                self.sync_snapshot(label, index, entries).await
            }
            HistoryEvent::Load {
                url,
                icon_url,
                length,
                ..
            } => {
                self.content_loaded(label, url, length as i32, icon_url)
                    .await
            }
            HistoryEvent::LoadFinished {
                url,
                title,
                icon_url,
            } => {
                self.on_page_load_finished_history(label, url, title, icon_url)
                    .await
            }
        }
    }

    /// on_page_load 完成时的历史校准（队列内执行，与前端 content_loaded 串行化）
    async fn on_page_load_finished_history(
        &self,
        label: &str,
        url: String,
        title: String,
        icon_url: String,
    ) -> Result<(), StateError> {
        let needs_id = self.tabs.sync_by_url(label, url.clone(), 0).await;
        let id = self
            .save_navigation_log(NavigationLog {
                url: url.clone(),
                title,
                icon_url,
                ..Default::default()
            })
            .await?;
        if needs_id {
            self.tabs.replace_history(label, id, url, 0).await;
        }
        self.tabs.set_redirecting(label, false).await;
        Ok(())
    }

    pub async fn parse_keyword(&self, keyword: &str) -> Option<Url> {
        let pool = self.db.get().await;
        let public_suffix = get_public_suffix_cached(&pool).await.ok();
        parse_keyword(public_suffix, keyword).await
    }

    pub async fn maximize(&self) -> Result<(), StateError> {
        self.window.maximize()?;

        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn unmaximize(&self) -> Result<(), StateError> {
        self.window.unmaximize()?;

        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn focus(&self) -> Result<(), StateError> {
        if !self.is_focused.set(true).await {
            return Ok(());
        }

        self.mainview.reparent(&self.window)?;

        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn blur(&self) -> Result<(), StateError> {
        if !self.is_focused.set(false).await {
            return Ok(());
        }

        let label = self.label.get().await;
        if !label.is_empty() {
            self.tabs.top(&label, &self.window).await?;
        }

        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn back(&self) -> Result<(), StateError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        let label = self.label.get().await;
        if label.is_empty() {
            return Ok(());
        }

        if self.tabs.back(&label).await {
            self.change_tab_loading_state(&label, true).await?;
        }

        Ok(())
    }

    pub async fn forward(&self) -> Result<(), StateError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        let label = self.label.get().await;
        if label.is_empty() {
            return Ok(());
        }

        if self.tabs.forward(&label).await {
            self.change_tab_loading_state(&label, true).await?;
        }

        Ok(())
    }

    pub async fn go(&self, index: usize) -> Result<(), StateError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        let label = self.label.get().await;
        if label.is_empty() {
            return Ok(());
        }

        if self.tabs.go(&label, index).await {
            self.change_tab_loading_state(&label, true).await?;
        }

        Ok(())
    }

    pub async fn reload(&self) -> Result<(), StateError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        let label = self.label.get().await;
        if label.is_empty() {
            return Ok(());
        }

        self.tabs.reload(&label).await;
        self.change_tab_loading_state(&label, true).await
    }

    pub async fn incognito(&self) -> Result<(), TabError> {
        if self.incognito.get().await {
            // 退出无痕模式
            self.tabs.close_incognito().await?;
            self.db.close_memory().await?;
            self.incognito.set(false).await;
            self.next_tab().await?;
        } else {
            // 进入无痕模式
            self.incognito.set(true).await;
            self.db.migrate_memory().await?;
            self.label.clear().await;
        }
        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn fullscreen(&self) -> Result<(), FrameworkError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        self.fullscreen_changed(!self.window.is_fullscreen()?).await
    }

    pub async fn switch_tab(&self, label: &str) -> Result<(), FrameworkError> {
        self.tabs.top(label, &self.window).await?;
        self.label.set(label.to_string()).await;
        Ok(())
    }

    pub async fn query_navigation_log(
        &self,
        keyword: String,
        page_token: PageToken,
    ) -> Result<QueryLogResponse, DatabaseError> {
        let pool = self.db.get().await;
        Ok(query_log(&pool, &keyword, page_token).await?)
    }

    pub async fn update_star(&self, id: i64) -> Result<(), DatabaseError> {
        let pool = self.db.get().await;
        update_log_star(&pool, id).await?;
        Ok(())
    }

    pub async fn get_state(&self, the_label: Option<&str>) -> Result<BrowserState, StateError> {
        let label = self.label.get().await;
        let mut state = self
            .tabs
            .get_state(the_label.unwrap_or(label.as_str()))
            .await
            .unwrap_or(BrowserState::default());

        state.maximized = self.window.is_maximized()?;
        state.focus = self.is_focused.get().await;
        state.incognito = self.incognito.get().await;

        Ok(state)
    }

    pub async fn fullscreen_changed(&self, is_fullscreen: bool) -> Result<(), FrameworkError> {
        self.window.set_fullscreen(is_fullscreen)?;
        let scale_factor = self.window.scale_factor()?;
        let mut web_size = self.window.inner_size()?.to_logical::<f64>(scale_factor);
        if !is_fullscreen {
            web_size.height -= Webview::TITLE_HEIGHT;
        }
        self.tabs.set_size(web_size).await;
        self.tabs
            .set_position(if is_fullscreen {
                LogicalPosition::new(0., 0.)
            } else {
                LogicalPosition::new(0., Webview::TITLE_HEIGHT)
            })
            .await;
        Ok(())
    }

    pub async fn leave_picture_in_picture(&self, label: &str) -> Result<(), StateError> {
        self.blur().await?;
        self.switch_tab(label).await?;
        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn focus_link(&self, url: String) -> Result<(), StateError> {
        let mut state = self.get_state(None).await?;
        state.title = FOCUS_LINK_TITLE.to_string();
        state.url = url;
        self.state_changed(Some(state)).await
    }

    pub async fn blur_link(&self) -> Result<(), StateError> {
        self.state_changed(None).await
    }

    pub async fn click_link(&self, url: String) -> Result<(), StateError> {
        let mut state = self.get_state(None).await?;
        state.url = url;
        state.loading = true;
        self.state_changed(Some(state)).await
    }

    pub async fn darkreader(&self) -> Result<(), StateError> {
        let label = self.label.get().await;
        if label.is_empty() {
            return Ok(());
        }

        let enable = self.tabs.darkreader(&label).await?;
        let state = self.get_state(None).await?;
        if let Ok(url) = Url::parse(&state.url)
            && let Some(host) = url.host_str()
        {
            let pool = self.db.get().await;
            let host = host.to_string();
            async_runtime::spawn(async move {
                if enable {
                    if let Err(e) = delete_blacklist(&pool, &host).await {
                        error!("删除 darkreader 黑名单 {host} 失败: {e}");
                    }
                } else if let Err(e) = save_blacklist(&pool, &host).await {
                    error!("保存 darkreader 黑名单 {host} 失败: {e}");
                }
            });
        }
        self.state_changed(Some(state)).await
    }

    pub async fn devtools(&self) {
        let label = self.label.get().await;
        if label.is_empty() {
            return;
        }

        self.tabs.devtools(&label).await;
    }

    pub async fn print(&self) -> Result<(), FrameworkError> {
        let label = self.label.get().await;
        if label.is_empty() {
            return Ok(());
        }

        self.tabs.print(&label).await
    }

    /// 重新聚焦webview
    pub async fn focus_changed(&self) -> Result<bool, FrameworkError> {
        let mut last_focus_changed = self.last_focus_changed.lock().await;
        if last_focus_changed.elapsed().as_millis() < 150 {
            return Ok(false);
        }

        let label = self.label.get().await;
        if self.is_focused.get().await || label.is_empty() {
            self.mainview.set_focus()?;
        } else {
            self.tabs.set_focus(&label).await?;
        }
        *last_focus_changed = Instant::now();

        Ok(true)
    }

    /// 根据系统主题更新窗口和 webview 背景色
    pub async fn update_theme(&self, theme: Theme) {
        let is_dark = matches!(theme, Theme::Dark);
        let bg = bg_color(is_dark);
        let _ = self.window.set_background_color(Some(bg));
        let _ = self.mainview.set_background_color(Some(bg));
        let label = self.label.get().await;
        let _ = self.tabs.set_background_color(&label, bg).await;
    }

    fn init_mainview() -> WebviewBuilder<Wry> {
        tauri::webview::WebviewBuilder::new(
            Webview::MAINVIEW_LABEL,
            WebviewUrl::App(Default::default()),
        )
        .auto_resize()
        .transparent(true)
        .zoom_hotkeys_enabled(false)
        .focused(true)
        .devtools(cfg!(debug_assertions))
    }

    async fn create_tab(&self, url: &Url, _active: bool) -> Result<TabId, FrameworkError> {
        let tab = Tab::new(&self.window, url, self.incognito.get().await)?;
        let label = tab.label().to_string();
        self.label.set(label.clone()).await;
        self.tabs.insert(label.clone(), tab).await;
        Ok(label)
    }

    async fn save_navigation_log(&self, log: NavigationLog) -> Result<i64, DatabaseError> {
        let pool = self.db.get().await;
        Ok(save_log(&pool, log).await?)
    }

    async fn get_icon_data_url(&self, icon_url: &str) -> Result<String, IconError> {
        let pool = self.db.get().await;
        get_icon_data_url(&pool, icon_url).await
    }

    async fn get_cached_icon(&self, url: &str) -> Option<String> {
        let pool = self.db.get().await;
        get_cached_icon(&pool, url).await
    }

    async fn change_tab_loading_state(&self, label: &str, loading: bool) -> Result<(), StateError> {
        self.tabs.set_loading(label, loading).await;

        if self.is_current_tab(label).await {
            self.state_changed(None).await?;
        }

        Ok(())
    }

    async fn state_changed(&self, state: Option<BrowserState>) -> Result<(), StateError> {
        let mut state = if let Some(state) = state {
            state
        } else {
            self.get_state(None).await?
        };

        // 在 emit 之前查询 icon 而不影响 state 原始数据
        if state.icon_url.is_empty()
            && !state.url.is_empty()
            && state.url.starts_with("http")
            && let Some(data_url) = self.get_cached_icon(&state.url).await
        {
            state.icon_url = data_url;
        } else if state.icon_url.starts_with("http")
            && let Ok(data_url) = self.get_icon_data_url(&state.icon_url).await
        {
            state.icon_url = data_url;
        }

        if state.title.is_empty() {
            state.title = LOADING_TITLE.to_string();
        }

        self.window
            .emit_to(Webview::MAINVIEW_LABEL, "state-changed", state)?;
        Ok(())
    }

    async fn darkreader_auto_switch(&self, label: &str, state: &mut BrowserState) {
        let enable = if let Ok(url) = Url::parse(&state.url)
            && let Some(host) = url.host_str()
        {
            let pool = self.db.get().await;
            darkreader::switch(&pool, host).await
        } else {
            true
        };

        if let Err(e) = self.tabs.set_darkreader(label, enable).await {
            error!("切换darkreader失败：{e}");
        } else {
            state.darkreader = enable;
        }
    }
}

pub trait BrowserExt {
    fn browser(&self) -> State<'_, Browser>;
}

impl<T: Manager<Wry>> BrowserExt for T {
    fn browser(&self) -> State<'_, Browser> {
        self.state::<Browser>()
    }
}
