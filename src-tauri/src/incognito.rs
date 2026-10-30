//! 无痕模式领域状态（由 TabService 持有）。
//!
//! 进入/退出/路由的顺序不变量内聚于此模块；UI 编排（焦点、主视图、
//! 浮动 Tab）由 Browser 门面负责，但必须遵守本模块文档中的调用顺序。

use log::error;
use tauri::async_runtime::Mutex;

use crate::{
    database::Database,
    error::TabError,
    icon::clear_cache as clear_icon_cache,
    log::clear_cache as clear_log_cache,
    state::Boolean,
    tab::{TabId, TabMap},
    tab_service::TabService,
};

/// 无痕模式领域状态：模式标志、无痕侧 Tab 存储、进入前的当前 tab，
/// 以及进入/退出/按模式路由的完整顺序不变量——全部内聚于此模块。
///
/// 由 TabService 持有；UI 编排（焦点、主视图、浮动 Tab）仍由 Browser 门面
/// 负责，但必须遵守本模块文档中的调用顺序。
pub struct Incognito {
    active: Boolean,
    /// 无痕侧 Tab 存储（与普通侧 TabMap 分离，避免切换时相互干扰）
    tabs: TabMap,
    /// 进入无痕模式前的当前 tab，退出时恢复
    pre_active: Mutex<Option<TabId>>,
}

impl Incognito {
    pub fn new() -> Self {
        Self {
            active: Boolean::default(),
            tabs: TabMap::new(),
            pre_active: Mutex::new(None),
        }
    }

    /// 供历史落库的缓存隔离查询无痕状态
    pub async fn is_active(&self) -> bool {
        self.active.get().await
    }

    /// 无痕侧存储（供跨模式查表与窗口级广播遍历）
    pub(crate) fn tabs(&self) -> &TabMap {
        &self.tabs
    }

    /// 当前模式对应的 TabMap（无痕侧或普通侧）
    pub(crate) async fn current_map<'a>(&'a self, normal: &'a TabMap) -> &'a TabMap {
        if self.active.get().await {
            &self.tabs
        } else {
            normal
        }
    }

    /// 查找指定 label 所在的 TabMap（先普通侧后无痕侧）
    pub(crate) async fn map_for<'a>(
        &'a self,
        normal: &'a TabMap,
        label: &str,
    ) -> Option<&'a TabMap> {
        if normal.contains(label).await {
            Some(normal)
        } else if self.tabs.contains(label).await {
            Some(&self.tabs)
        } else {
            None
        }
    }

    /// 按显式模式标志选表（create_tab 等创建路径在标志捕获后使用）
    pub(crate) fn map_for_mode<'a>(&'a self, normal: &'a TabMap, mode: bool) -> &'a TabMap {
        if mode { &self.tabs } else { normal }
    }

    /// 指定 id 的历史记录是否已在对应模式的 tab 中打开
    pub(crate) async fn any_open(
        &self,
        normal: &TabMap,
        id: i64,
        mode: bool,
    ) -> Option<(TabId, usize)> {
        let map = self.map_for_mode(normal, mode);
        map.any_open(id).await
    }

    /// 进入无痕模式，顺序：建立内存库（失败则整体未进入，不留任何状态）→
    /// 记住当前 tab（退出时恢复）→ 翻转标志 → 清空当前指向（无痕侧尚无 tab）。
    pub async fn enter(&self, db: &Database, tabs: &TabService) -> Result<(), TabError> {
        db.migrate_memory().await?;
        *self.pre_active.lock().await = Some(tabs.current.get().await);
        self.active.set(true).await;
        tabs.current.clear().await;
        Ok(())
    }

    /// 退出无痕模式。顺序不变量（调用方不得打乱）：
    /// 1. 关闭全部无痕 tab 并**排空**其历史消费者（等待在途事件处理完），
    ///    确保内存库被关闭前无残留写入落回持久库
    /// 2. 关闭内存库，无痕数据随连接销毁
    /// 3. 翻转标志后再清空全局缓存：此后新调用均以普通侧 key 查询持久库，
    ///    不会把内存库结果写回 (true, …) 缓存条目（在途调用已在第 1 步排空）
    /// 4. 恢复进入无痕模式前的 tab（不存在则回退到普通侧相邻 tab）
    pub async fn exit(&self, db: &Database, tabs: &TabService) -> Result<(), TabError> {
        let removed = self.tabs.keys().await;
        for label in &removed {
            self.tabs.close(label).await?;
        }
        tabs.drain_consumers(&removed).await;

        db.close_memory().await?;

        self.active.set(false).await;
        clear_log_cache().await;
        clear_icon_cache().await;

        self.restore(tabs).await
    }

    /// 恢复进入无痕前的 tab（不存在则回退到相邻 tab）
    async fn restore(&self, tabs: &TabService) -> Result<(), TabError> {
        if let Some(prev) = self.pre_active.lock().await.take()
            && !prev.is_empty()
        {
            if let Err(e) = tabs.switch_tab(&prev).await {
                error!("恢复无痕前标签失败：{e}");
            } else {
                return Ok(());
            }
        }
        tabs.next_tab().await
    }
}
