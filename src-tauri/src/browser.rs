use crate::{
    DB_NAME, IsMainView,
    error::*,
    get_db_url,
    icon::get_icon_data_url,
    log::{NavigationLog, QueryLogResponse, get_id, get_url, query_log, save_log, update_log_star},
    page::PageToken,
    public_suffix::get_public_suffix_cached,
    shortcut::{self, GlobalShortcutExt},
    state::{BrowserState, Focused},
    tab::{Tab, TabIndex, TabMap},
    task,
    url::parse_keyword,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tauri::{
    App, AppHandle, Emitter as _, LogicalPosition, Manager, State, Url, Webview, WebviewBuilder,
    WebviewUrl, Window, WindowEvent, Wry, async_runtime, window::Color,
};
use tauri_plugin_window_state::{AppHandleExt, StateFlags, WindowExt};

pub struct Browser {
    db: SqlitePool,
    window: Window,
    mainview: Webview,
    label: TabIndex,
    tabs: TabMap,
    is_focused: Focused,
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

            const WIDTH: f64 = 800.;
            const HEIGHT: f64 = 600.;

            let window = tauri::window::WindowBuilder::new(app, "main")
                .title("白洞")
                .inner_size(WIDTH, HEIGHT)
                .min_inner_size(WIDTH, HEIGHT)
                .decorations(false)
                .background_color(Color(29, 35, 42, 0))
                .build()?;

            window.restore_state(StateFlags::all())?;

            let mainview = window.add_child(
                Self::init_mainview(),
                LogicalPosition::new(0., 0.),
                window.inner_size()?,
            )?;

            let db = SqlitePool::connect(get_db_url(app)?).await?;

            let state = Browser {
                db,
                window,
                mainview,
                label: TabIndex::new(),
                tabs: TabMap::new(),
                is_focused: Focused::new(),
            };
            app.manage(state);

            task::setup()?;
            shortcut::setup(app)?;

            Ok(())
        })
    }

    pub async fn on_new_window(app_handle: &AppHandle, url: &Url) -> Result<(), TabError> {
        let browser = app_handle.browser();

        browser.open_tab_by_url(url, true).await?;
        browser.state_changed(None).await?;
        Ok(())
    }

    pub async fn on_window_event(window: &Window, event: &WindowEvent) -> Result<(), WindowError> {
        match event {
            WindowEvent::Resized(_) => {
                let browser = window.browser();
                browser.resize().await?;
                browser.state_changed(None).await?;
            }
            WindowEvent::Focused(true) => {
                let shortcut = window.global_shortcut();
                shortcut.resume().await?;
            }
            WindowEvent::Focused(false) => {
                let shortcut = window.global_shortcut();
                shortcut.pause().await?;
            }
            WindowEvent::Destroyed => {
                window.app_handle().save_window_state(StateFlags::all())?;
            }
            _ => {}
        }

        Ok(())
    }

    pub async fn resize(&self) -> Result<(), FrameworkError> {
        let scale_factor = self.window.scale_factor()?;
        let mut web_size = self.window.inner_size()?.to_logical::<f64>(scale_factor);
        web_size.height -= Webview::TITLE_HEIGHT;
        self.tabs.set_size(web_size).await;
        Ok(())
    }

    pub async fn close_tab(&self) -> Result<(), TabError> {
        let label = self.label.get().await;
        self.tabs.close(&label).await?;
        self.label.set(String::new()).await;

        if let Some(near_label) = self.tabs.near(&label).await {
            self.switch_tab(&near_label).await?;
        }

        Ok(())
    }

    pub async fn open_tab_by_url(&self, url: &Url, _active: bool) -> Result<(), TabError> {
        if let Some(id) = get_id(&self.db, url.as_str()).await
            && let Some((label, index)) = self.tabs.any_open(id).await
        {
            self.tabs.go(&label, index).await;
            self.switch_tab(&label).await?;
        } else {
            self.create_tab(url, true).await?;
        }
        Ok(())
    }

    pub async fn open_tab(&self, id: i64) -> Result<(), TabError> {
        if let Some((label, index)) = self.tabs.any_open(id).await {
            self.tabs.go(&label, index).await;
            self.switch_tab(&label).await?;
        } else if let Some(url) = get_url(&self.db, id).await {
            self.create_tab(&Url::parse(&url)?, true).await?;
        }
        Ok(())
    }

    pub async fn next_tab(&self) -> Result<bool, TabError> {
        let label = self.label.get().await;
        let mut is_switch = false;
        if let Some(next_label) = self.tabs.next(&label).await {
            self.switch_tab(&next_label).await?;
            is_switch = true;
        }

        Ok(is_switch)
    }

    pub async fn is_current_tab(&self, label: &str) -> bool {
        self.label.eq(label).await
    }

    pub async fn change_tab_title(&self, label: &str, title: String) {
        self.tabs.set_title(label, title).await;
    }

    pub async fn change_tab_icon(&self, label: &str, icon_url: String) {
        self.tabs.set_icon(label, icon_url).await;
    }

    pub async fn change_tab_loading_state(
        &self,
        label: &str,
        loading: bool,
    ) -> Result<(), StateError> {
        self.tabs.set_loading(label, loading).await;
        self.push_history_state(label).await
    }

    pub async fn push_history_state(&self, label: &str) -> Result<(), StateError> {
        let state = self.get_state(Some(label)).await?;
        if self.is_current_tab(label).await {
            self.state_changed(Some(state.clone())).await?;
        }

        if state.url == "about:blank" {
            // 非http协议的空白页面，关闭当前tab
            self.tabs.close(label).await?;
            return Ok(());
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

    pub async fn parse_keyword(&self, keyword: &str) -> Option<Url> {
        let public_suffix = get_public_suffix_cached(&self.db).await.ok();
        parse_keyword(public_suffix, keyword).await
    }

    pub fn maximize(&self) -> Result<(), FrameworkError> {
        Ok(self.window.maximize()?)
    }

    pub fn unmaximize(&self) -> Result<(), FrameworkError> {
        Ok(self.window.unmaximize()?)
    }

    pub async fn focus(&self) -> Result<bool, FrameworkError> {
        if !self.is_focused.set(true).await {
            return Ok(false);
        }

        self.mainview.reparent(&self.window)?;
        Ok(true)
    }

    pub async fn blur(&self) -> Result<bool, FrameworkError> {
        if !self.is_focused.set(false).await {
            return Ok(false);
        }

        let label = self.label.get().await;
        if label.is_empty() {
            return Ok(true);
        }
        self.tabs.top(&label, &self.window).await?;
        Ok(true)
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

    pub async fn save_navigation_log(&self, log: NavigationLog) -> Result<i64, LogError> {
        Ok(save_log(&self.db, log).await?)
    }

    pub async fn query_navigation_log(
        &self,
        keyword: String,
        page_token: PageToken,
    ) -> Result<QueryLogResponse, LogError> {
        Ok(query_log(&self.db, &keyword, page_token).await?)
    }

    pub async fn update_star(&self, id: i64) -> Result<(), LogError> {
        update_log_star(&self.db, id).await?;
        Ok(())
    }

    pub async fn get_state(&self, the_label: Option<&str>) -> Result<BrowserState, StateError> {
        let maximized = self.window.is_maximized()?;
        let label = self.label.get().await;
        let focus = self.is_focused.get().await;

        let mut state = self
            .tabs
            .get_state(the_label.unwrap_or(label.as_str()))
            .await
            .unwrap_or(BrowserState::default());
        if !state.icon_url.is_empty()
            && let Ok(icon_url) = self.get_icon_data_url(&state.icon_url).await
        {
            state.icon_url = icon_url;
        }
        state.maximized = maximized;
        state.focus = focus;
        Ok(state)
    }

    pub async fn state_changed(&self, state: Option<BrowserState>) -> Result<(), StateError> {
        let state = if let Some(state) = state {
            state
        } else {
            self.get_state(None).await?
        };

        self.window
            .emit_to(Webview::MAINVIEW_LABEL, "state-changed", state)?;
        Ok(())
    }

    fn init_mainview() -> WebviewBuilder<Wry> {
        tauri::webview::WebviewBuilder::new(
            Webview::MAINVIEW_LABEL,
            WebviewUrl::App(Default::default()),
        )
        .auto_resize()
        .transparent(true)
        .zoom_hotkeys_enabled(false)
        .devtools(cfg!(debug_assertions))
    }

    async fn create_tab(&self, url: &Url, _active: bool) -> Result<(), FrameworkError> {
        let tab = Tab::new(&self.window, url)?;
        self.is_focused.set(false).await;
        let label = tab.label().to_string();
        self.label.set(label.clone()).await;
        self.tabs.insert(label, tab).await;
        Ok(())
    }

    async fn switch_tab(&self, label: &str) -> Result<(), FrameworkError> {
        self.is_focused.set(false).await;
        self.tabs.top(label, &self.window).await?;
        self.label.set(label.to_string()).await;
        Ok(())
    }

    async fn get_icon_data_url(&self, url: &str) -> Result<String, IconError> {
        get_icon_data_url(&self.db, url).await
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
