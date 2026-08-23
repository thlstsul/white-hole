use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use log::{error, info};
use sqlx::SqlitePool;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager as _, Url, Webview, Window, Wry,
    async_runtime::{self, Mutex},
    webview::{DownloadEvent, NewWindowResponse, PageLoadPayload},
    window::Color,
};
use tauri_plugin_notification::NotificationExt;
use uuid::Uuid;

use crate::{
    browser::{Browser, BrowserExt as _},
    darkreader,
    error::{DatabaseError, FrameworkError, StateError, TabError},
    history::{HistoryEvent, HistorySnapshotEntry},
    log::{NavigationLog, save_log},
    state::BrowserState,
    tab::{Tab, TabId, TabIndex, TabMap},
};

/// 单个 tab 的历史事件消费者：严格按入队顺序应用该 tab 的事件
pub(crate) async fn consume_history(
    label: TabId,
    mut rx: async_runtime::Receiver<HistoryEvent>,
    app_handle: AppHandle,
) {
    while let Some(event) = rx.recv().await {
        let browser = app_handle.browser();
        if let Err(e) = browser.tabs.apply_history_event(&label, event).await {
            error!("{label} 应用历史事件失败：{e}");
        }
    }
}

/// 新 tab 注册窗口期（webview 已创建、TabMap 尚未插入）的事件暂存：
/// 按 label 隔离，create_tab 插入后一次性补投（FIFO）
#[derive(Default)]
struct PendingEvents {
    map: HashMap<TabId, VecDeque<HistoryEvent>>,
}

impl PendingEvents {
    fn new() -> Self {
        Self::default()
    }

    /// 暂存一条事件（同一 label 内保持入队顺序）
    fn push(&mut self, label: TabId, event: HistoryEvent) {
        self.map.entry(label).or_default().push_back(event);
    }

    /// 取出某 label 的全部暂存事件（FIFO；一次性——第二次调用返回空）
    fn drain(&mut self, label: &str) -> Vec<HistoryEvent> {
        self.map
            .remove(label)
            .map(|mut events| events.drain(..).collect())
            .unwrap_or_default()
    }

    /// 清理已关闭 tab 的暂存事件
    fn clear(&mut self, label: &str) {
        self.map.remove(label);
    }
}

/// 标签领域服务：持有全部 tab 状态（TabMap / 当前 tab / 历史事件路由），
/// 承担标签生命周期、历史镜像同步与每 tab 命令。
/// 与 Browser 解耦：不直接持有窗口与数据库，需要基础设施（DB、UI 发射、窗口）时
/// 经 AppHandle 回调 Browser（发射与落库仍收敛在 Browser）。
pub struct TabService {
    map: TabMap,
    current: TabIndex,
    /// 进入无痕模式前的当前 tab，退出时恢复（与 current 同属当前状态管理）
    pre_incognito: Mutex<Option<TabId>>,
    /// 新 tab 注册窗口期（webview 已创建、TabMap 尚未插入）的历史事件暂存：
    /// create_tab 插入后补投，避免首个文档的快照/图标事件被丢弃
    pending_history: Mutex<PendingEvents>,
    /// 每 tab 历史事件消费者的任务句柄：关闭无痕前等待其排空，
    /// 防止在途事件在内存库被关闭后把无痕数据写进持久库
    consumers: Mutex<HashMap<TabId, async_runtime::JoinHandle<()>>>,
    app_handle: AppHandle,
}

impl TabService {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            map: TabMap::new(),
            current: TabIndex::new(),
            pre_incognito: Mutex::new(None),
            pending_history: Mutex::new(PendingEvents::new()),
            consumers: Mutex::new(HashMap::new()),
            app_handle,
        }
    }

    /// 主窗口（单窗口应用，创建/置顶 tab 需要；窗口已销毁时返回错误而非 panic）
    fn window(&self) -> Result<Window, FrameworkError> {
        self.app_handle
            .get_window("main")
            .ok_or(FrameworkError::Tauri(tauri::Error::WindowNotFound))
    }

    fn browser(&self) -> tauri::State<'_, Browser> {
        self.app_handle.browser()
    }

    async fn db(&self) -> Arc<SqlitePool> {
        self.browser().db().await
    }

    /// 通知主视图状态变更（发射统一收敛在 Browser::state_changed）
    async fn emit(&self, state: Option<BrowserState>) -> Result<(), StateError> {
        self.browser().state_changed(state).await
    }

    async fn save_navigation_log(&self, log: NavigationLog) -> Result<i64, DatabaseError> {
        let pool = self.db().await;
        Ok(save_log(&pool, log).await?)
    }

    // ============ 当前 tab ============

    pub async fn current(&self) -> TabId {
        self.current.get().await
    }

    /// 进入无痕模式：记住当前 tab（退出时恢复），并清空当前指向
    pub async fn enter_incognito(&self) {
        *self.pre_incognito.lock().await = Some(self.current.get().await);
        self.current.clear().await;
    }

    /// 退出无痕模式：恢复进入无痕前的 tab（不存在则回退到相邻 tab）
    pub async fn restore_previous_tab(&self) -> Result<(), TabError> {
        match self.pre_incognito.lock().await.take() {
            Some(prev) if !prev.is_empty() => match self.switch_tab(&prev).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    error!("恢复无痕前标签失败：{e}");
                    self.next_tab().await
                }
            },
            _ => self.next_tab().await,
        }
    }

    // ============ 生命周期 ============

    pub async fn create_tab(&self, url: &Url, incognito: bool) -> Result<TabId, FrameworkError> {
        let window = self.window()?;
        // 每个 tab 独立的历史事件 FIFO 队列与专属消费者（严格按入队顺序应用）；
        // 队列先于 webview 创建，避免首个文档事件因队列未就绪而丢失
        let label = Uuid::now_v7().to_string();
        let (history_queue, history_rx) = async_runtime::channel::<HistoryEvent>(1024);
        let app_handle = self.app_handle.clone();
        let consumer = async_runtime::spawn(consume_history(label.clone(), history_rx, app_handle));
        self.consumers.lock().await.insert(label.clone(), consumer);

        let tab = match Tab::new(&window, &label, url, incognito, history_queue.clone()) {
            Ok(tab) => tab,
            Err(e) => {
                // webview 创建失败：清理已注册的消费者与暂存事件，
                // 避免 JoinHandle 与待补投事件随失败 tab 泄漏
                self.consumers.lock().await.remove(&label);
                self.pending_history.lock().await.clear(&label);
                return Err(e);
            }
        };
        self.current.set(label.clone()).await;
        // 持锁完成"插入 + 补投"：与 enqueue_history 的"查表 + 暂存"串行化，
        // 注册窗口期的事件要么在补投前入暂存、要么在插入后直接发送——不丢失、不乱序
        let mut pending = self.pending_history.lock().await;
        self.map.insert(label.clone(), tab).await;
        for event in pending.drain(&label) {
            // 新建队列为空，发送不会阻塞；持锁保证先于插入后到达的直接发送
            let _ = history_queue.send(event).await;
        }
        drop(pending);
        Ok(label)
    }

    pub async fn close_tab(&self) -> Result<(), TabError> {
        let label = self.current.get().await;
        self.map.close(&label).await?;
        self.pending_history.lock().await.clear(&label);
        // 等待该 tab 的消费者任务排空（通道已随 Tab 销毁关闭）后再返回：
        // 保证其历史事件全部处理完，避免退出无痕关内存库后仍有在途写入落回持久库
        if let Some(handle) = self.consumers.lock().await.remove(&label) {
            let _ = handle.await;
        }
        self.current.clear().await;
        if let Some(near_label) = self.map.near(&label).await {
            self.switch_tab(&near_label).await?;
        }
        self.emit(None).await?;
        Ok(())
    }

    /// 关闭全部无痕 tab，并**排空**其历史消费者（等待在途事件处理完）后返回：
    /// 调用方随后才关闭内存库，此时已无在途消费者会把无痕数据写进持久库
    pub async fn close_incognito(&self) -> Result<(), FrameworkError> {
        let removed = self.map.close_incognito().await?;
        {
            let mut pending = self.pending_history.lock().await;
            for label in &removed {
                pending.clear(label);
            }
        }
        // 等待无痕 tab 的消费者任务退出：通道已随 Tab 销毁关闭，
        // 消费者处理完剩余事件后自然结束——确保内存库关闭前无在途写入
        let handles: Vec<_> = {
            let mut consumers = self.consumers.lock().await;
            removed
                .iter()
                .filter_map(|label| consumers.remove(label))
                .collect()
        };
        for handle in handles {
            let _ = handle.await;
        }
        Ok(())
    }

    pub async fn switch_tab(&self, label: &str) -> Result<(), FrameworkError> {
        let window = self.window()?;
        self.map.top(label, &window).await?;
        self.current.set(label.to_string()).await;
        Ok(())
    }

    pub async fn next_tab(&self) -> Result<(), TabError> {
        let label = self.current.get().await;
        if let Some(next_label) = self.map.next(&label).await {
            self.switch_tab(&next_label).await?;
            self.emit(None).await?;
        }
        Ok(())
    }

    pub async fn near_tab(&self) -> Result<(), TabError> {
        let label = self.current.get().await;
        if let Some(near_label) = self.map.near(&label).await {
            self.switch_tab(&near_label).await?;
            self.emit(None).await?;
        }
        Ok(())
    }

    pub async fn top(&self, label: &str) -> Result<(), FrameworkError> {
        let window = self.window()?;
        self.map.top(label, &window).await
    }

    pub async fn any_open(&self, id: i64, incognito: bool) -> Option<(TabId, usize)> {
        self.map.any_open(id, incognito).await
    }

    pub async fn go_to(&self, label: &str, index: usize) -> bool {
        self.map.go(label, index).await
    }

    pub async fn insert_history(&self, label: &str, id: i64, url: String, length: usize) {
        self.map.insert_history(label, id, url, length).await;
    }

    pub async fn get_state(&self, label: &str) -> Result<BrowserState, FrameworkError> {
        self.map.get_state(label).await
    }

    // ============ 每 tab 命令 ============

    pub async fn back(&self) -> Result<(), StateError> {
        let label = self.current.get().await;
        if self.map.back(&label).await {
            self.change_tab_loading_state(&label, true).await?;
        }
        Ok(())
    }

    pub async fn forward(&self) -> Result<(), StateError> {
        let label = self.current.get().await;
        if self.map.forward(&label).await {
            self.change_tab_loading_state(&label, true).await?;
        }
        Ok(())
    }

    pub async fn go(&self, index: usize) -> Result<(), StateError> {
        let label = self.current.get().await;
        if self.map.go(&label, index).await {
            self.change_tab_loading_state(&label, true).await?;
        }
        Ok(())
    }

    pub async fn reload(&self) -> Result<(), StateError> {
        let label = self.current.get().await;
        self.map.reload(&label).await;
        self.change_tab_loading_state(&label, true).await
    }

    pub async fn devtools(&self) {
        let label = self.current.get().await;
        if label.is_empty() {
            return;
        }
        self.map.devtools(&label).await;
    }

    pub async fn print(&self) -> Result<(), FrameworkError> {
        let label = self.current.get().await;
        if label.is_empty() {
            return Ok(());
        }
        self.map.print(&label).await
    }

    /// 切换当前 tab 的暗色模式，返回新状态
    pub async fn toggle_darkreader(&self, label: &str) -> Result<bool, tauri::Error> {
        self.map.darkreader(label).await
    }

    pub async fn set_focus(&self, label: &str) -> Result<(), FrameworkError> {
        self.map.set_focus(label).await
    }

    pub async fn set_size(&self, size: LogicalSize<f64>) {
        self.map.set_size(size).await;
    }

    pub async fn set_position(&self, position: LogicalPosition<f64>) {
        self.map.set_position(position).await;
    }

    pub async fn set_background_color(
        &self,
        label: &str,
        color: Color,
    ) -> Result<(), tauri::Error> {
        self.map.set_background_color(label, color).await
    }

    // ============ 历史镜像同步 ============

    /// 将历史事件入队到该 tab 自己的 FIFO 队列（队列定义在 Tab 内，随 tab 创建/销毁；
    /// 由该 tab 专属消费者按序应用；队列满时等待背压，且只阻塞本 tab）
    pub async fn enqueue_history(&self, label: impl Into<String>, event: HistoryEvent) {
        let label = label.into();
        let sender = {
            let mut pending = self.pending_history.lock().await;
            // 持锁查表 + 暂存原子化：与 create_tab 的"插入 + 补投"在同一把锁内串行化，
            // 杜绝"查表未命中 → 插入补投 → 才入暂存"的丢失竞态
            if let Some(sender) = self.map.history_queue(&label).await {
                Some((sender, event))
            } else if self.app_handle.get_webview(&label).is_some() {
                // tab 尚未注册（webview 已创建、TabMap 未插入）：暂存待补投
                pending.push(label, event);
                None
            } else {
                // webview 已销毁（tab 已关闭）：丢弃迟到的 IPC/加载事件
                None
            }
        };
        if let Some((sender, event)) = sender {
            let _ = sender.send(event).await;
        }
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
        let needs_id = self.map.sync_by_url(label, url.clone(), 0).await;
        let id = self
            .save_navigation_log(NavigationLog {
                url: url.clone(),
                title,
                icon_url,
                ..Default::default()
            })
            .await?;
        if needs_id {
            self.map.replace_history(label, id, url, 0).await;
        }
        self.map.set_redirecting(label, false).await;
        Ok(())
    }

    pub async fn content_loaded(
        &self,
        label: &str,
        url: String,
        length: i32,
        icon_url: String,
    ) -> Result<(), StateError> {
        self.map.set_icon(label, icon_url).await;

        let mut state = self.browser().get_state(Some(label)).await?;
        self.darkreader_auto_switch(label, &mut state).await;

        if self.current.eq(label).await {
            self.emit(Some(state.clone())).await?;
        }

        let length = length as usize;
        let needs_id = self.map.sync_by_url(label, url.clone(), length).await;
        // 无条件落库：content_loaded 携带页面真实图标（此时 set_icon 已生效），
        // 若不落库，URL 已存在于历史栈（needs_id=false）时图标永远不会保存。
        // 用前端上报的权威 url 落库：state.url 取自同步前的镜像，无 Navigation API
        // 的 WebView（快照不上报）上仍指向上一个文档，会把新页面的标题/图标写进旧记录
        let id = self
            .save_navigation_log(NavigationLog {
                url: url.clone(),
                title: state.title.clone(),
                icon_url: state.icon_url.clone(),
                ..Default::default()
            })
            .await?;
        if needs_id {
            // 仅当 sync_by_url 插入了新条目（占位 id=-1）时才回填历史栈 id，
            // 避免 URL 命中旧条目时误覆盖旧条目的 id
            self.map.replace_history(label, id, url, length).await;
        }

        Ok(())
    }

    pub async fn on_page_load(&self, label: &str, loading: bool) -> Result<(), StateError> {
        if loading {
            // 页面已在加载中又触发 Started = 重定向链（302/meta refresh/reload）
            let redirecting = self.map.is_loading(label).await;
            self.map.start_loading(label).await;
            self.map.set_redirecting(label, redirecting).await;
            // 真实加载已开始，loading 交由 PageLoadEvent::Finished 清理
            self.map.set_nav_pending(label, false).await;
            return Ok(());
        }

        self.map.set_loading(label, loading).await;

        let state = self.browser().get_state(Some(label)).await?;
        if self.current.eq(label).await {
            self.emit(Some(state.clone())).await?;
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
        let label = self.current.get().await;
        if label.is_empty() {
            return;
        }

        self.map.set_loading(&label, loading).await;
    }

    /// Navigation API 权威快照对账：全量重建镜像，新 key 或 URL 变更（replaceState）
    /// 条目落库并回填 id，最后刷新当前标签页 UI 状态（back/forward 按钮）
    pub async fn sync_snapshot(
        &self,
        label: &str,
        index: usize,
        entries: Vec<HistorySnapshotEntry>,
    ) -> Result<(), StateError> {
        let needs_id = self.map.sync_snapshot(label, index, entries).await;
        // 同文档导航（pushState/popstate）或 bfcache 恢复不触发页面加载事件，
        // 快照到达即导航完成，清掉 back/forward/go 置起的 loading
        if self.map.take_nav_pending(label).await {
            self.map.set_loading(label, false).await;
        }
        // 快照不含 title/icon，当前条目（replaceState 改 URL）落库时从标签页取
        let state = self.map.get_state(label).await?;
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
            self.map.backfill_history(label, pos, id, url).await;
        }
        // 补写加载期间被跳过的标题：同文档导航（back/forward/go/reload 到已存在条目）
        // 由快照确认而非 PageLoad Finished，且已存在条目不在 needs_id 中不会重存，
        // 此处用快照对账后的权威 URL 落库，避免标题静默丢失
        if let Some(title) = self.map.take_pending_title(label).await {
            let url = state.url.clone();
            if !url.is_empty() && url != "about:blank" {
                self.save_navigation_log(NavigationLog {
                    url,
                    title,
                    icon_url: state.icon_url.clone(),
                    ..Default::default()
                })
                .await?;
            }
        }
        if self.current.eq(label).await {
            self.emit(Some(state)).await?;
        }
        Ok(())
    }

    /// 标记/清除加载状态并同步 UI；导航开始时置 nav_pending 等待确认
    pub async fn change_tab_loading_state(
        &self,
        label: &str,
        loading: bool,
    ) -> Result<(), StateError> {
        self.map.set_loading(label, loading).await;
        if loading {
            // 等待页面加载事件（跨文档）或 Navigation API 快照（同文档/bfcache）确认导航完成
            self.map.set_nav_pending(label, true).await;
        }

        if self.current.eq(label).await {
            self.emit(None).await?;
        }

        Ok(())
    }

    pub async fn change_tab_title(&self, label: &str, title: String) -> Result<(), StateError> {
        self.map.set_title(label, title.clone()).await;

        let mut state = self.browser().get_state(Some(label)).await?;
        self.darkreader_auto_switch(label, &mut state).await;

        if self.current.eq(label).await {
            self.emit(Some(state.clone())).await?;
        }

        // 跨文档导航进行中（loading=true）：TitleChanged 事件先于历史镜像更新到达，
        // current_url() 仍指向旧文档，此时落库会把新标题写进旧 URL 的记录，造成标题错位；
        // 改为记录 pending_title，待快照对账后用权威 URL 补写（同文档导航无
        // PageLoad Finished 事件，快照是唯一确认点，不能依赖 on_page_load_finished_history）。
        // 同文档改标题（SPA document.title，loading=false）时镜像可信，照常落库。
        if self.map.is_loading(label).await {
            self.map.set_pending_title(label, title).await;
        } else {
            self.save_navigation_log(state.into()).await?;
        }
        Ok(())
    }

    /// 根据站点黑名单自动切换暗色模式
    async fn darkreader_auto_switch(&self, label: &str, state: &mut BrowserState) {
        let enable = if let Ok(url) = Url::parse(&state.url)
            && let Some(host) = url.host_str()
        {
            let pool = self.db().await;
            darkreader::switch(&pool, host).await
        } else {
            true
        };

        if let Err(e) = self.map.set_darkreader(label, enable).await {
            error!("切换darkreader失败：{e}");
        } else {
            state.darkreader = enable;
        }
    }
}

// ============ Webview 事件回调（由 Tab::new 注册到 WebviewBuilder） ============

pub(crate) fn on_new_window(app_handle: &AppHandle, url: Url) -> NewWindowResponse<Wry> {
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

pub(crate) fn on_document_title_changed(webview: Webview, title: String) {
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

pub(crate) fn on_page_load(webview: Webview, payload: PageLoadPayload) {
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

pub(crate) fn on_download(webview: Webview, event: DownloadEvent) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn load(url: &str) -> HistoryEvent {
        HistoryEvent::Load {
            url: url.to_string(),
            icon_url: String::new(),
            length: 0,
        }
    }

    #[test]
    fn pending_drain_is_fifo_and_one_shot() {
        let mut pending = PendingEvents::new();
        pending.push("a".into(), load("u1"));
        pending.push("a".into(), load("u2"));
        pending.push("a".into(), load("u3"));

        let drained = pending.drain("a");
        assert_eq!(drained.len(), 3);
        // 同一 label 保持入队顺序（FIFO）
        for (event, expected) in drained.into_iter().zip(["u1", "u2", "u3"]) {
            match event {
                HistoryEvent::Load { url, .. } => assert_eq!(url, expected),
                _ => panic!("unexpected event type"),
            }
        }
        // 一次性补投：第二次取出为空
        assert!(pending.drain("a").is_empty());
    }

    #[test]
    fn pending_labels_are_isolated() {
        let mut pending = PendingEvents::new();
        pending.push("a".into(), load("u1"));
        pending.push("b".into(), load("v1"));

        assert_eq!(pending.drain("a").len(), 1);
        assert_eq!(pending.drain("b").len(), 1);
        assert!(pending.drain("c").is_empty());
    }

    #[test]
    fn pending_clear_removes_events() {
        let mut pending = PendingEvents::new();
        pending.push("a".into(), load("u1"));
        pending.clear("a");
        assert!(pending.drain("a").is_empty());
        // 清理不存在的 label 无害
        pending.clear("nope");
    }
}
