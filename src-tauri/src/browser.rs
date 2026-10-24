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

use crate::{
    IsMainView,
    darkreader::{delete_blacklist, save_blacklist},
    database::Database,
    error::*,
    floating_tab::{self, FloatingTab, FloatingTabState},
    fullscreen::Fullscreen,
    history::HistoryEvent,
    icon::{clear_cache as clear_icon_cache, get_cached_icon, get_icon_data_url},
    log::{
        NavigationLog, QueryLogResponse, clear_cache as clear_log_cache, get_id, get_url,
        query_log, save_log, touch_log, update_log_star,
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
    /// 浮动 Tab 状态（数据与布局内聚在 floating_tab 模块）
    pub(crate) floating: FloatingTabState,
    is_focused: Boolean,
    incognito: Boolean,
    is_client: Boolean,
    /// 全屏状态机（F11 窗口全屏 + webview 视频全屏的标志与窗口切换）
    fullscreen: Fullscreen,
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
            let fullscreen = Fullscreen::new(window.clone())?;

            let state = Browser {
                db,
                window,
                mainview,
                tabs: TabService::new(app.handle().clone()),
                floating: FloatingTabState::default(),
                is_focused: Boolean::default(),
                incognito: Boolean::default(),
                is_client: Boolean::default(),
                fullscreen,
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

    /// 供 TabService 查询无痕状态（历史落库的缓存隔离依赖此标志）
    pub(crate) async fn is_incognito(&self) -> bool {
        self.incognito.get().await
    }

    /// 计算 webview 应占用的位置与尺寸，基于窗口客户区。
    /// 全屏时窗口已由 set_fullscreen 铺满显示器（其触发的 WM_SIZE → resize
    /// 中 inner_size 即屏幕尺寸），无需另取显示器大小。
    /// `deduct_title` 为 true 时减去自绘标题栏高度
    /// （tab webview），false 时占满整个窗口（mainview 标签栏 UI）。
    fn webview_rect(
        &self,
        is_fullscreen: bool,
        deduct_title: bool,
    ) -> Result<(LogicalPosition<f64>, LogicalSize<f64>), FrameworkError> {
        let scale_factor = self.window.scale_factor()?;
        let mut size = self.window.inner_size()?.to_logical::<f64>(scale_factor);
        let position = if is_fullscreen {
            LogicalPosition::new(0., 0.)
        } else if deduct_title {
            size.height -= Webview::TITLE_HEIGHT;
            LogicalPosition::new(0., Webview::TITLE_HEIGHT)
        } else {
            LogicalPosition::new(0., 0.)
        };
        Ok((position, size))
    }

    pub async fn resize(&self) -> Result<(), StateError> {
        let is_fullscreen = self.fullscreen.is_active().await;
        // tab 始终跟随窗口尺寸（全屏时位 (0,0)，普通时减标题栏），
        // 不能因窗口小于初始尺寸而跳过——否则退出全屏后 tab 保持全屏尺寸盖住标题栏
        let (position, web_size) = self.webview_rect(is_fullscreen, true)?;
        if !self.tabs.current().await.is_empty() {
            self.tabs.set_position(position).await;
            self.tabs.set_size(web_size).await;
        }
        // mainview（标签栏 UI）占满整个窗口/屏幕，不依赖 auto_resize
        let (main_pos, main_size) = self.webview_rect(is_fullscreen, false)?;
        if let Err(e) = self.mainview.set_position(main_pos) {
            error!("mainview set_position 失败：{e}");
        }
        if let Err(e) = self.mainview.set_size(main_size) {
            error!("mainview set_size 失败：{e}");
        }
        // floating_layout 期望完整窗口尺寸（内部再扣标题栏），
        // 复用 main_size 而非已扣除标题栏的 web_size，避免高度双重扣除
        self.floating.resize(main_size).await;

        self.emit(None).await?;
        Ok(())
    }

    /// 返回是否实际执行了 tab 操作，供调用方决定是否需要跟进 focus_changed
    pub async fn close_tab(&self) -> Result<bool, TabError> {
        if self.is_focused.get().await {
            return Ok(false);
        }

        self.tabs.close_tab().await?;
        Ok(true)
    }

    /// 同步读取 Ctrl 是否按下（避免异步竞态）
    fn ctrl_pressed(&self) -> bool {
        use ::hotkey::{Code, HotkeyManagerExt as _};
        let hotkey = self.window.app_handle().hotkey();
        hotkey.is_pressed(Code::ControlLeft) || hotkey.is_pressed(Code::ControlRight)
    }

    pub async fn open_tab_by_url(&self, url: &Url) -> Result<(), TabError> {
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
        if let Some(id) = get_id(&pool, incognito, url.as_str()).await
            && let Some((label, index)) = self.tabs.any_open(id, incognito).await
        {
            self.tabs.go_to(&label, index).await;
            self.tabs.switch_tab(&label).await?;
            self.emit(None).await?;
            // 已打开的 tab 也刷新对应浏览记录的 last_time
            if let Err(e) = touch_log(&pool, id).await {
                log::error!("刷新浏览记录 last_time 失败：{e}");
            }
        } else {
            let label = self.tabs.create_tab(url, incognito).await?;
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
        self.floating.raise(&self.window).await;
        Ok(())
    }

    pub async fn open_tab(&self, id: i64) -> Result<(), TabError> {
        if self.ctrl_pressed() {
            let incognito = self.incognito.get().await;
            if let Some(url) = get_url(self.db.get().await.as_ref(), incognito, id).await {
                self.open_floating_tab(&Url::parse(&url)?).await?;
            }
            return Ok(());
        }

        let pool = self.db.get().await;
        let incognito = self.incognito.get().await;
        self.is_focused.set(false).await;
        if let Some((label, index)) = self.tabs.any_open(id, incognito).await {
            self.tabs.go_to(&label, index).await;
            self.tabs.switch_tab(&label).await?;
            self.emit(None).await?;
            if let Err(e) = touch_log(pool.as_ref(), id).await {
                log::error!("刷新浏览记录 last_time 失败：{e}");
            }
        } else if let Some(url) = get_url(pool.as_ref(), incognito, id).await {
            let label = self.tabs.create_tab(&Url::parse(&url)?, incognito).await?;
            let mut state = self.get_state(None).await?;
            state.url = url.clone();
            self.emit(Some(state.clone())).await?;
            let mut log: NavigationLog = state.into();
            log.title.clear();
            let id = self.save_navigation_log(log).await?;
            self.tabs.insert_history(&label, id, url, 1).await;
        }

        self.focus_changed().await?;
        self.floating.raise(&self.window).await;
        Ok(())
    }

    /// 返回是否实际执行了 tab 操作，供调用方决定是否需要跟进 focus_changed
    pub async fn next_tab(&self) -> Result<bool, TabError> {
        if self.is_focused.get().await {
            return Ok(false);
        }

        self.tabs.next_tab().await?;
        Ok(true)
    }

    /// 返回是否实际执行了 tab 操作，供调用方决定是否需要跟进 focus_changed
    pub async fn near_tab(&self) -> Result<bool, TabError> {
        if self.is_focused.get().await {
            return Ok(false);
        }

        self.tabs.near_tab().await?;
        Ok(true)
    }

    /// 打开浮动 Tab：关闭已有 → 创建 webview（同窗口 add_child）→ 注入 JS 控制栏
    pub async fn open_floating_tab(&self, url: &Url) -> Result<(), FrameworkError> {
        // 1. 计算浮动 webview 位置和尺寸（主窗口内逻辑坐标，无锁）
        let scale = self.window.scale_factor()?;
        let win_size = self.window.inner_size()?.to_logical::<f64>(scale);
        let (position, size) = floating_tab::layout(win_size);

        // 2. 构建 webview（无锁）
        let incognito = self.incognito.get().await;
        let (label, builder) = floating_tab::webview_builder(&self.window, url, incognito)?;
        let webview = self.window.add_child(builder, position, size)?;

        // 3. 原子操作：关闭已有 + 存储新 Tab（单次锁，消除并发竞态窗口）
        let new_floating = FloatingTab {
            label: label.clone(),
            url: url.to_string(),
            webview,
        };
        if let Some(old) = self.floating.replace(new_floating).await {
            let _ = old.webview.close();
        }

        Ok(())
    }

    /// 关闭浮动 Tab：销毁 webview，清理状态，恢复焦点到主窗口
    pub async fn close_floating_tab(&self) -> Result<(), FrameworkError> {
        if let Some(f) = self.floating.take().await {
            f.webview.close()?;
            // FT-11：销毁浮动 Tab 后，将焦点交还给主窗口（WebView2 未显式聚焦时
            // 可能停留在浮动 Tab 最后交互的区域，需手动恢复）
            let _ = self.window.set_focus();
        }
        Ok(())
    }

    /// 提升浮动 Tab 为常规 Tab：原子 take（锁内解析 URL）→ 关闭浮动 → 创建常规 Tab
    pub async fn promote_floating_tab(&self) -> Result<(), TabError> {
        // 同一锁内解析 URL 并原子取出；解析失败时浮动 Tab 仍存活
        let Some((url, f)) = self.floating.take_validated(Url::parse).await? else {
            return Ok(());
        };
        f.webview.close()?;
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

        self.emit(None).await
    }

    pub async fn unmaximize(&self) -> Result<(), StateError> {
        self.window.unmaximize()?;

        self.emit(None).await
    }

    /// 返回主视图状态是否实际变化，供调用方决定是否需要跟进 focus_changed
    pub async fn focus(&self) -> Result<bool, StateError> {
        if !self.is_focused.set(true).await {
            return Ok(false);
        }

        self.mainview.reparent(&self.window)?;
        self.floating.raise(&self.window).await;

        self.emit(None).await?;
        Ok(true)
    }

    /// 返回主视图状态是否实际变化，供调用方决定是否需要跟进 focus_changed
    pub async fn blur(&self) -> Result<bool, StateError> {
        if !self.is_focused.set(false).await {
            return Ok(false);
        }

        let label = self.tabs.current().await;
        if !label.is_empty() {
            self.tabs.top(&label).await?;
        }
        self.floating.raise(&self.window).await;

        self.emit(None).await?;
        Ok(true)
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
            // 翻转标志后再清空全局缓存：此后新调用均以普通侧 key 查询持久库，
            // 不会把内存库结果写回 (true, …) 缓存条目（在途调用已由 close_incognito 排空）
            clear_log_cache().await;
            clear_icon_cache().await;
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

        let is_fullscreen = !self.fullscreen.is_active().await;
        self.fullscreen.toggle_window(is_fullscreen).await
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
        Ok(update_log_star(&pool, id).await?)
    }

    pub async fn get_state(&self, the_label: Option<&str>) -> Result<BrowserState, StateError> {
        let label = self.tabs.current().await;
        let mut state = self
            .tabs
            .get_state(the_label.unwrap_or(label.as_str()))
            .await
            .unwrap_or_default();

        state.maximized = self.window.is_maximized()?;
        state.focus = self.is_focused.get().await;
        state.incognito = self.incognito.get().await;
        state.is_client = self.is_client.get().await;

        Ok(state)
    }

    /// webview 元素（视频）全屏上报：窗口切换由 wry 原生处理并维护
    /// 最大化快照（见 Fullscreen::webview_changed），此处只负责布局收敛。
    pub async fn webview_fullscreen_changed(
        &self,
        is_fullscreen: bool,
    ) -> Result<(), FrameworkError> {
        self.fullscreen.webview_changed(is_fullscreen).await?;
        // wry 的原生切换不产生我们的 WM_SIZE，显式收敛一次布局
        // （StateError 是 FrameworkError 超集，无法向上传播，
        // 有意 best-effort 吞错记日志）
        if let Err(e) = self.resize().await {
            error!("视频全屏后布局收敛失败：{e}");
        }
        Ok(())
    }

    pub async fn leave_picture_in_picture(&self, label: &str) -> Result<(), StateError> {
        self.blur().await?;
        self.tabs.switch_tab(label).await?;
        self.emit(None).await
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

    /// Navigation API navigate 事件上报：导航真实发起时由引擎确认目的地 URL，
    /// 设置乐观 URL 让 UI 即时反映（乐观 URL 唯一来源，label 取自发起导航的 webview）
    pub async fn navigate_started(&self, label: &str, url: String) -> Result<(), StateError> {
        self.tabs.set_optimistic_url(label, url).await
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
                        error!("删除 darkreader 黑名单 {host} 失败：{e}");
                    }
                } else if let Err(e) = save_blacklist(&pool, &host).await {
                    error!("保存 darkreader 黑名单 {host} 失败：{e}");
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
        let incognito = self.incognito.get().await;
        Ok(save_log(&pool, incognito, log).await?)
    }

    async fn get_icon_data_url(&self, icon_url: &str) -> Result<String, IconError> {
        let pool = self.db.get().await;
        let incognito = self.incognito.get().await;
        get_icon_data_url(&pool, incognito, icon_url).await
    }

    async fn get_cached_icon(&self, url: &str) -> Option<String> {
        let pool = self.db.get().await;
        let incognito = self.incognito.get().await;
        get_cached_icon(&pool, incognito, url).await
    }

    /// 发射状态到主视图（TabService 经此通知 UI，发射与图标查询收敛于此）
    pub(crate) async fn emit(&self, state: Option<BrowserState>) -> Result<(), StateError> {
        let mut state = match state {
            Some(state) => state,
            None => self.get_state(None).await?,
        };

        if state.icon_url.is_empty()
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
