use std::sync::Arc;

use log::error;
use sqlx::SqlitePool;
use tauri::{
    App, Emitter as _, LogicalPosition, LogicalSize, Manager, State, Theme, Url, Webview,
    WebviewBuilder, WebviewUrl, Window, Wry,
    async_runtime::{self, Mutex},
};
use tauri_plugin_window_state::{StateFlags, WindowExt};
use tokio::time::Instant;
use uuid::Uuid;

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
    tab_service::{TabService, on_floating_new_window, on_floating_page_load},
    task,
    url::parse_keyword,
    user_agent::get_user_agent,
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

    /// 浮动 Tab 位置和尺寸计算（复用于 resize / open_floating_tab）
    fn floating_layout(
        &self,
        window_size: LogicalSize<f64>,
    ) -> (LogicalPosition<f64>, LogicalSize<f64>) {
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

    pub async fn resize(&self) -> Result<(), StateError> {
        let scale_factor = self.window.scale_factor()?;
        let mut web_size = self.window.inner_size()?.to_logical::<f64>(scale_factor);
        let window_height = web_size.height;
        if !(self.tabs.current().await.is_empty()
            || web_size.height < HEIGHT
            || web_size.width < WIDTH)
        {
            web_size.height -= Webview::TITLE_HEIGHT;
            self.tabs.set_size(web_size).await;
        }

        // —— 浮动 Tab 跟随缩放 ——
        {
            let mut floating = self.tabs.floating_tab.lock().await;
            if let Some(ref mut f) = *floating {
                let (pos, size) =
                    self.floating_layout(LogicalSize::new(web_size.width, window_height));
                if let Err(e) = f.webview.set_size(size) {
                    error!("浮动 Tab set_size 失败：{e}");
                }
                if let Err(e) = f.webview.set_position(pos) {
                    error!("浮动 Tab set_position 失败：{e}");
                }
            }
        }

        self.emit(None).await?;
        Ok(())
    }

    pub async fn close_tab(&self) -> Result<(), TabError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        self.tabs.close_tab().await
    }

    /// 同步读取 Ctrl 是否按下（避免异步竞态）
    fn ctrl_pressed(&self) -> bool {
        use ::hotkey::{Code, HotkeyManagerExt as _};
        let hotkey = self.window.app_handle().hotkey();
        hotkey.is_pressed(Code::ControlLeft) || hotkey.is_pressed(Code::ControlRight)
    }

    pub async fn open_tab_by_url(&self, url: &Url, _active: bool) -> Result<(), TabError> {
        if self.ctrl_pressed() {
            self.open_floating_tab(url).await?;
            return Ok(());
        }

        self.open_tab_regular(url).await
    }

    /// 以常规 Tab 方式打开 URL（不检测 Ctrl，供 promote 等需固定行为的路径复用）
    async fn open_tab_regular(&self, url: &Url) -> Result<(), TabError> {
        let pool = self.db.get().await;
        let incognito = self.incognito.get().await;
        self.is_focused.set(false).await;
        if let Some(id) = get_id(&pool, url.as_str()).await
            && let Some((label, index)) = self.tabs.any_open(id, incognito).await
        {
            self.tabs.go_to(&label, index).await;
            self.tabs.switch_tab(&label).await?;
            self.emit(None).await?;
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
            self.emit(Some(state.clone())).await?;
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
        if self.ctrl_pressed() {
            if let Some(url) = get_url(self.db.get().await.as_ref(), id).await {
                self.open_floating_tab(&Url::parse(&url)?).await?;
            }
            return Ok(());
        }

        let incognito = self.incognito.get().await;
        self.is_focused.set(false).await;
        if let Some((label, index)) = self.tabs.any_open(id, incognito).await {
            self.tabs.go_to(&label, index).await;
            self.tabs.switch_tab(&label).await?;
            self.emit(None).await?;
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
            self.emit(Some(state.clone())).await?;
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

    /// 打开浮动 Tab：关闭已有 → 创建 webview（同窗口 add_child）→ 注入 JS 控制栏
    pub async fn open_floating_tab(&self, url: &Url) -> Result<(), FrameworkError> {
        // 1. 计算浮动 webview 位置和尺寸（主窗口内逻辑坐标，无锁）
        let scale = self.window.scale_factor()?;
        let win_size = self.window.inner_size()?.to_logical::<f64>(scale);
        let (position, size) = self.floating_layout(win_size);

        // 2. 构建 webview（无锁）
        let label = format!("floating-{}", Uuid::now_v7());
        let app_handle = self.window.app_handle().clone();
        let is_dark = matches!(self.window.theme()?, Theme::Dark);
        let incognito = self.incognito.get().await;
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
            .on_download(crate::tab_service::on_download);

        let webview = self.window.add_child(builder, position, size)?;

        // 3. 原子操作：关闭已有 + 存储新 Tab（单次锁，消除并发竞态窗口）
        let new_floating = crate::tab_service::FloatingTab {
            label: label.clone(),
            url: url.to_string(),
            webview,
        };
        let mut lock = self.tabs.floating_tab.lock().await;
        // 关闭已有浮动 Tab（如有）
        if let Some(f) = lock.take() {
            let _ = f.webview.close();
        }
        *lock = Some(new_floating);

        Ok(())
    }

    /// 关闭浮动 Tab：销毁 webview，清理状态，恢复焦点到主窗口
    pub async fn close_floating_tab(&self) -> Result<(), FrameworkError> {
        let floating = self.tabs.floating_tab.lock().await.take();
        if let Some(f) = floating {
            f.webview.close()?;
            // FT-11：销毁浮动 Tab 后，将焦点交还给主窗口（WebView2 未显式聚焦时
            // 可能停留在浮动 Tab 最后交互的区域，需手动恢复）
            let _ = self.window.set_focus();
        }
        Ok(())
    }

    /// 提升浮动 Tab 为常规 Tab：原子 take → 校验 URL → 关闭浮动 → 创建常规 Tab
    pub async fn promote_floating_tab(&self) -> Result<(), TabError> {
        // 同一锁内先校验 URL、再原子 take；解析失败时浮动 Tab 仍存活
        let (url, webview) = {
            let mut lock = self.tabs.floating_tab.lock().await;
            let Some(f) = lock.as_ref() else {
                return Ok(());
            };
            let url = Url::parse(&f.url)?;
            let f = lock.take().expect("上一步已确认存在");
            (url, f.webview)
        };
        webview.close()?;
        let _ = self.window.set_focus();

        self.open_tab_regular(&url).await?;
        Ok(())
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

        self.emit(None).await?;
        Ok(())
    }

    pub async fn unmaximize(&self) -> Result<(), StateError> {
        self.window.unmaximize()?;

        self.emit(None).await?;
        Ok(())
    }

    pub async fn focus(&self) -> Result<(), StateError> {
        if !self.is_focused.set(true).await {
            return Ok(());
        }

        self.mainview.reparent(&self.window)?;

        // 浮动 Tab 重新置顶
        if let Some(ref floating) = *self.tabs.floating_tab.lock().await {
            let _ = floating.webview.reparent(&self.window);
        }

        self.emit(None).await?;
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

        // 浮动 Tab 重新置顶（top 内已补，此处兜底 mainview reparent 路径）
        if let Some(ref floating) = *self.tabs.floating_tab.lock().await {
            let _ = floating.webview.reparent(&self.window);
        }

        self.emit(None).await?;
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
            self.close_floating_tab()
                .await
                .inspect_err(|e| error!("关闭浮动 Tab 失败：{e}"))
                .ok();
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
        self.emit(None).await?;
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
        self.emit(None).await?;
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
        self.emit(None).await?;
        Ok(())
    }

    pub async fn focus_link(&self, url: String) -> Result<(), StateError> {
        let mut state = self.get_state(None).await?;
        state.title = FOCUS_LINK_TITLE.to_string();
        state.url = url;
        self.emit(Some(state)).await
    }

    pub async fn blur_link(&self) -> Result<(), StateError> {
        self.emit(None).await
    }

    pub async fn click_link(&self, url: String) -> Result<(), StateError> {
        // 将乐观 URL 存入当前 tab，在真实导航追上之前，get_state 会自动使用此值覆盖，
        // 确保点击链接后 UI 立即反映新 URL 和加载状态，避免网络延迟时无反应
        let label = self.tabs.current().await;
        if !label.is_empty() {
            self.tabs.set_optimistic_url(&label, url).await;
        }
        let state = self.get_state(Some(&label)).await?;
        self.emit(Some(state)).await
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
        self.emit(Some(state)).await
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
    pub(crate) async fn emit(&self, state: Option<BrowserState>) -> Result<(), StateError> {
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
