use std::sync::Arc;

use log::error;
use sqlx::SqlitePool;
use tauri::{
    App, Emitter as _, LogicalPosition, Manager, State, Theme, Url, Webview, WebviewBuilder,
    WebviewUrl, Window, Wry,
    async_runtime::{self, Mutex},
};
use tauri_plugin_window_state::{StateFlags, WindowExt};
use tokio::time::Instant;

use crate::{
    IsMainView,
    darkreader::{delete_blacklist, save_blacklist},
    database::Database,
    error::*,
    history::HistoryEvent,
    icon::{get_cached_icon, get_icon_data_url},
    log::{
        NavigationLog, QueryLogResponse, get_id, get_url, query_log, save_log, touch_log,
        update_log_star,
    },
    page::PageToken,
    public_suffix::get_public_suffix_cached,
    state::{Boolean, BrowserState},
    tab::bg_color,
    tab_service::TabService,
    task,
    url::parse_keyword,
};

const WIDTH: f64 = 800.;
const HEIGHT: f64 = 600.;
const FOCUS_LINK_TITLE: &str = "点击链接：";
const LOADING_TITLE: &str = "正在加载……";

/// 窗口编排 + 应用门面：持有窗口/主视图/数据库与标签领域服务，
/// 负责窗口级操作、状态发射与 IPC 委托；标签领域逻辑全部收敛在 TabService。
pub struct Browser {
    db: Database,
    window: Window,
    mainview: Webview,
    /// 标签领域服务（标签生命周期 / 历史镜像同步 / 每 tab 命令）
    pub(crate) tabs: TabService,
    is_focused: Boolean,
    incognito: Boolean,
    is_client: Boolean,
    last_focus_changed: Mutex<Instant>,
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

            let state = Browser {
                db,
                window,
                mainview,
                tabs: TabService::new(app.handle().clone()),
                is_focused: Boolean::default(),
                incognito: Boolean::default(),
                is_client: Boolean::default(),
                last_focus_changed: Mutex::new(Instant::now()),
            };
            app.manage(state);

            task::setup()?;

            Ok(())
        })
    }

    /// 供 TabService 复用数据库连接池
    pub(crate) async fn db(&self) -> Arc<SqlitePool> {
        self.db.get().await
    }

    pub async fn resize(&self) -> Result<(), StateError> {
        let scale_factor = self.window.scale_factor()?;
        let mut web_size = self.window.inner_size()?.to_logical::<f64>(scale_factor);
        if !(self.tabs.current().await.is_empty()
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

        self.tabs.close_tab().await
    }

    pub async fn open_tab_by_url(&self, url: &Url, _active: bool) -> Result<(), TabError> {
        let pool = self.db.get().await;
        let incognito = self.incognito.get().await;
        self.is_focused.set(false).await;
        if let Some(id) = get_id(&pool, url.as_str()).await
            && let Some((label, index)) = self.tabs.any_open(id, incognito).await
        {
            self.tabs.go_to(&label, index).await;
            self.tabs.switch_tab(&label).await?;
            self.state_changed(None).await?;
            // 已打开的 tab 也刷新对应浏览记录的 last_time
            if let Err(e) = touch_log(&pool, id).await {
                log::error!("刷新浏览记录 last_time 失败: {e}");
            }
        } else {
            let label = self
                .tabs
                .create_tab(url, self.incognito.get().await)
                .await?;
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
            self.tabs.go_to(&label, index).await;
            self.tabs.switch_tab(&label).await?;
            self.state_changed(None).await?;
            // 已打开的 tab 也刷新对应浏览记录的 last_time
            if let Err(e) = touch_log(self.db.get().await.as_ref(), id).await {
                log::error!("刷新浏览记录 last_time 失败: {e}");
            }
        } else if let Some(url) = get_url(self.db.get().await.as_ref(), id).await {
            let label = self
                .tabs
                .create_tab(&Url::parse(&url)?, self.incognito.get().await)
                .await?;
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

        self.tabs.next_tab().await
    }

    pub async fn near_tab(&self) -> Result<(), TabError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        self.tabs.near_tab().await
    }

    /// 文档标题变更（由 WebviewBuilder::on_document_title_changed 触发）
    pub async fn change_tab_title(&self, label: &str, title: String) -> Result<(), StateError> {
        self.tabs.change_tab_title(label, title).await
    }

    /// 页面加载事件（由 WebviewBuilder::on_page_load 触发）
    pub async fn on_page_load(&self, label: &str, loading: bool) -> Result<(), StateError> {
        self.tabs.on_page_load(label, loading).await
    }

    pub async fn set_loading(&self, loading: bool) {
        self.tabs.set_loading(loading).await;
    }

    /// 将历史事件入队到该 tab 自己的 FIFO 队列（由 TabService 路由）
    pub async fn enqueue_history(&self, label: impl Into<String>, event: HistoryEvent) {
        self.tabs.enqueue_history(label, event).await
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

        let label = self.tabs.current().await;
        if !label.is_empty() {
            self.tabs.top(&label).await?;
        }

        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn back(&self) -> Result<(), StateError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        self.tabs.back().await
    }

    pub async fn forward(&self) -> Result<(), StateError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        self.tabs.forward().await
    }

    pub async fn go(&self, index: usize) -> Result<(), StateError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        self.tabs.go(index).await
    }

    pub async fn reload(&self) -> Result<(), StateError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        self.tabs.reload().await
    }

    pub async fn incognito(&self) -> Result<(), TabError> {
        if self.incognito.get().await {
            // 退出无痕模式：close_incognito 排空无痕 tab 的在途历史消费者，
            // 确保内存库被关闭前无残留写入落回持久库
            self.is_focused.set(false).await;
            self.tabs.close_incognito().await?;
            self.db.close_memory().await?;
            self.incognito.set(false).await;
            // 恢复进入无痕模式前的 tab（由 TabService 的 current 机制管理）
            self.tabs.restore_previous_tab().await?;
        } else {
            // 进入无痕模式
            self.incognito.set(true).await;
            self.is_focused.set(true).await;
            // 抬起主视图到 tab 之上，保持 is_focused ⇔ mainview 顶层不变量
            self.mainview.reparent(&self.window)?;
            self.db.migrate_memory().await?;
            self.tabs.enter_incognito().await;
        }
        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn http_client(&self) -> Result<(), StateError> {
        if self.is_client.get().await {
            self.is_focused.set(false).await;
            self.is_client.set(false).await;
            // 恢复当前 tab 到顶层（与 blur() 一致）
            let label = self.tabs.current().await;
            if !label.is_empty() {
                self.tabs.top(&label).await?;
            }
        } else {
            self.is_client.set(true).await;
            self.is_focused.set(true).await;
            // 抬起主视图到 tab 之上，保持 is_focused ⇔ mainview 顶层不变量
            self.mainview.reparent(&self.window)?;
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
        let label = self.tabs.current().await;
        let mut state = self
            .tabs
            .get_state(the_label.unwrap_or(label.as_str()))
            .await
            .unwrap_or(BrowserState::default());

        state.maximized = self.window.is_maximized()?;
        state.focus = self.is_focused.get().await;
        state.incognito = self.incognito.get().await;
        state.is_client = self.is_client.get().await;

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
        self.tabs.switch_tab(label).await?;
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
        let label = self.tabs.current().await;
        if label.is_empty() {
            return Ok(());
        }

        let enable = self.tabs.toggle_darkreader(&label).await?;
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
        self.tabs.devtools().await;
    }

    pub async fn print(&self) -> Result<(), FrameworkError> {
        self.tabs.print().await
    }

    /// 重新聚焦webview
    pub async fn focus_changed(&self) -> Result<bool, FrameworkError> {
        let mut last_focus_changed = self.last_focus_changed.lock().await;
        if last_focus_changed.elapsed().as_millis() < 150 {
            return Ok(false);
        }

        let label = self.tabs.current().await;
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
        let label = self.tabs.current().await;
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

    /// 发射状态到主视图（TabService 经此通知 UI，发射与图标查询收敛于此）
    pub(crate) async fn state_changed(
        &self,
        state: Option<BrowserState>,
    ) -> Result<(), StateError> {
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
}

pub trait BrowserExt {
    fn browser(&self) -> State<'_, Browser>;
}

impl<T: Manager<Wry>> BrowserExt for T {
    fn browser(&self) -> State<'_, Browser> {
        self.state::<Browser>()
    }
}
