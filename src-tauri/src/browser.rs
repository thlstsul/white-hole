use std::time::Duration;

use crate::{
    IsMainView,
    database::{DB_NAME, Database},
    error::*,
    icon::get_icon_data_url,
    log::{NavigationLog, QueryLogResponse, get_id, get_url, query_log, save_log, update_log_star},
    page::PageToken,
    public_suffix::get_public_suffix_cached,
    state::{Boolean, BrowserState},
    tab::{Tab, TabIndex, TabMap},
    task,
    url::parse_keyword,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tauri::{
    App, Emitter as _, LogicalPosition, Manager, State, Url, Webview, WebviewBuilder, WebviewUrl,
    Window, Wry,
    async_runtime::{self, Mutex},
    window::Color,
};
use tauri_plugin_window_state::{StateFlags, WindowExt};
use tokio::time::Instant;

const WIDTH: f64 = 800.;
const HEIGHT: f64 = 600.;

pub struct Browser {
    db: Database,
    window: Window,
    mainview: Webview,
    label: TabIndex,
    tabs: TabMap,
    is_focused: Boolean,
    incognito: Boolean,
    last_focus_changed: Mutex<Instant>,
}

impl Browser {
    pub fn setup(app: &mut App) -> Result<(), SetupError> {
        async_runtime::block_on(async {
            let db_path = app.path().app_local_data_dir()?.join(DB_NAME);
            if !db_path.exists() {
                let options = SqliteConnectOptions::new()
                    .filename(db_path)
                    .create_if_missing(true)
                    .foreign_keys(true);

                let pool = SqlitePoolOptions::new().connect_with(options).await?;
                sqlx::migrate!("../migrations").run(&pool).await?;
            }

            let window = tauri::window::WindowBuilder::new(app, "main")
                .title("白洞")
                .inner_size(WIDTH, HEIGHT)
                .min_inner_size(WIDTH, HEIGHT)
                .decorations(false)
                .focused(true)
                .background_color(Color(29, 35, 42, 0))
                .build()?;

            window.restore_state(StateFlags::all())?;

            let mainview = window.add_child(
                Self::init_mainview(),
                LogicalPosition::new(0., 0.),
                window.inner_size()?,
            )?;

            let db = Database::new(app).await?;

            let state = Browser {
                db,
                window,
                mainview,
                label: TabIndex::new(),
                tabs: TabMap::new(),
                is_focused: Boolean::default(),
                incognito: Boolean::default(),
                last_focus_changed: Mutex::new(Instant::now()),
            };
            app.manage(state);

            task::setup()?;

            Ok(())
        })
    }

    pub async fn resize(&self) -> Result<(), StateError> {
        let scale_factor = self.window.scale_factor()?;
        let mut web_size = self.window.inner_size()?.to_logical::<f64>(scale_factor);
        if self.label.get().await.is_empty() || web_size.height < HEIGHT || web_size.width < WIDTH {
            // 无TAB或最小化后，不需要变更大小
            return Ok(());
        }

        web_size.height -= Webview::TITLE_HEIGHT;
        self.tabs.set_size(web_size).await;

        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn close_tab(&self) -> Result<(), TabError> {
        if self.is_focused.get().await {
            return Ok(());
        }

        let label = self.label.get().await;
        self.tabs.close(&label).await?;
        self.label.set(String::new()).await;

        if let Some(near_label) = self.tabs.near(&label).await {
            self.switch_tab(&near_label).await?;
        }

        self.state_changed(None).await?;
        Ok(())
    }

    pub async fn open_tab_by_url(&self, url: &Url, _active: bool) -> Result<(), TabError> {
        let pool = self.db.get().await;
        let incognito = self.incognito.get().await;
        if let Some(id) = get_id(&pool, url.as_str()).await
            && let Some((label, index)) = self.tabs.any_open(id, incognito).await
        {
            self.tabs.go(&label, index).await;
            self.switch_tab(&label).await?;
        } else {
            self.create_tab(url, true).await?;
        }
        self.is_focused.set(false).await;

        self.state_changed(None).await?;
        self.focus_changed().await?;
        Ok(())
    }

    pub async fn open_tab(&self, id: i64) -> Result<(), TabError> {
        let incognito = self.incognito.get().await;
        if let Some((label, index)) = self.tabs.any_open(id, incognito).await {
            self.tabs.go(&label, index).await;
            self.switch_tab(&label).await?;
        } else if let Some(url) = get_url(self.db.get().await.as_ref(), id).await {
            self.create_tab(&Url::parse(&url)?, true).await?;
        }
        self.is_focused.set(false).await;

        self.state_changed(None).await?;
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

    pub async fn is_current_tab(&self, label: &str) -> bool {
        self.label.eq(label).await
    }

    pub async fn change_tab_title(&self, label: &str, title: String) -> Result<(), StateError> {
        self.tabs.set_title(label, title).await;

        let state = self.get_state(None).await?;
        if self.is_current_tab(label).await {
            self.state_changed(Some(state.clone())).await?;
        }

        let log: NavigationLog = state.into();
        let id = self.save_navigation_log(log).await?;
        self.tabs.insert_history(label, id).await;
        Ok(())
    }

    pub async fn change_tab_icon(&self, label: &str, icon_url: String) -> Result<(), StateError> {
        self.tabs.set_icon(label, icon_url).await;

        let state = self.get_state(None).await?;
        if self.is_current_tab(label).await {
            self.state_changed(Some(state.clone())).await?;
        }
        self.save_navigation_log(state.into()).await?;
        Ok(())
    }

    pub async fn change_tab_loading_state(
        &self,
        label: &str,
        loading: bool,
    ) -> Result<(), StateError> {
        self.tabs.set_loading(label, loading).await;

        if self.is_current_tab(label).await {
            self.state_changed(None).await?;
        }

        Ok(())
    }

    pub async fn push_history_state(&self, label: &str) -> Result<(), StateError> {
        let state = self.get_state(Some(label)).await?;
        if self.is_current_tab(label).await {
            self.state_changed(Some(state.clone())).await?;
        }

        let log: NavigationLog = state.into();
        let id = self.save_navigation_log(log).await?;
        self.tabs.insert_history(label, id).await;

        Ok(())
    }

    pub async fn replace_history_state(&self, label: &str) -> Result<(), StateError> {
        let state = self.get_state(Some(label)).await?;
        if self.is_current_tab(label).await {
            self.state_changed(Some(state.clone())).await?;
        }

        let log: NavigationLog = state.into();
        let id = self.save_navigation_log(log).await?;
        self.tabs.replace_history(label, id).await;

        Ok(())
    }

    pub async fn hash_changed(&self, label: &str) -> Result<(), StateError> {
        let state = self.get_state(Some(label)).await?;
        if self.is_current_tab(label).await {
            self.state_changed(Some(state.clone())).await?;
        }

        let log: NavigationLog = state.into();
        let id = self.save_navigation_log(log).await?;
        self.tabs.insert_history(label, id).await;

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

    pub async fn back(&self) {
        if self.is_focused.get().await {
            return;
        }

        let label = self.label.get().await;
        if label.is_empty() {
            return;
        }

        self.tabs.back(&label).await;
    }

    pub async fn forward(&self) {
        if self.is_focused.get().await {
            return;
        }

        let label = self.label.get().await;
        if label.is_empty() {
            return;
        }

        self.tabs.forward(&label).await;
    }

    pub async fn go(&self, index: usize) {
        if self.is_focused.get().await {
            return;
        }

        let label = self.label.get().await;
        if label.is_empty() {
            return;
        }

        self.tabs.go(&label, index).await;
    }

    pub async fn reload(&self) {
        if self.is_focused.get().await {
            return;
        }

        let label = self.label.get().await;
        if label.is_empty() {
            return;
        }

        self.tabs.reload(&label).await;
    }

    pub async fn incognito(&self) -> Result<(), StateError> {
        if self.incognito.get().await {
            // 退出无痕模式
            self.tabs.close_incognito().await?;
            self.db.close_memory().await?;
            self.incognito.set(false).await;
        } else {
            // 进入无痕模式
            self.incognito.set(true).await;
            self.db.migrate_memory().await?;
            self.label.set(String::new()).await;
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

    /// 重新聚焦webview
    pub async fn focus_changed(&self) -> Result<bool, FrameworkError> {
        let now = Instant::now();
        let mut last_focus_changed = self.last_focus_changed.lock().await;
        if now.duration_since(*last_focus_changed) < Duration::from_millis(50) {
            return Ok(false);
        }

        if self.is_focused.get().await || self.label.get().await.is_empty() {
            self.mainview.set_focus()?;
        } else {
            self.tabs.set_focus(&self.label.get().await).await?;
        }
        *last_focus_changed = now;

        Ok(true)
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

    async fn create_tab(&self, url: &Url, _active: bool) -> Result<(), FrameworkError> {
        let tab = Tab::new(&self.window, url, self.incognito.get().await)?;
        let label = tab.label().to_string();
        self.label.set(label.clone()).await;
        self.tabs.insert(label, tab).await;
        Ok(())
    }

    async fn save_navigation_log(&self, log: NavigationLog) -> Result<i64, DatabaseError> {
        let pool = self.db.get().await;
        Ok(save_log(&pool, log).await?)
    }

    async fn get_icon_data_url(&self, url: &str) -> Result<String, IconError> {
        let pool = self.db.get().await;
        get_icon_data_url(&pool, url).await
    }

    async fn state_changed(&self, state: Option<BrowserState>) -> Result<(), StateError> {
        let mut state = if let Some(state) = state {
            state
        } else {
            self.get_state(None).await?
        };

        if !state.icon_url.is_empty()
            && let Ok(icon_url) = self.get_icon_data_url(&state.icon_url).await
        {
            state.icon_url = icon_url;
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
