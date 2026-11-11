//! 单个标签页的领域模型：webview 句柄、乐观 URL、历史镜像及事件队列。
//!
//! 历史镜像以 Navigation API 快照对账（快照只是同源片段，见
//! [`Tab::sync_snapshot`]），是 URL/标题落库与回退前进的权威来源。

use std::collections::HashMap as StdHashMap;
use std::ops::Deref;

use log::{error, info};
use scc::HashMap;
use tauri::{
    Error, LogicalPosition, LogicalSize, Manager as _, Theme, Webview, WebviewBuilder, WebviewUrl,
    Window,
    async_runtime::{self, RwLock},
    window::Color,
};
use url::Url;

use crate::{
    IsMainView as _,
    darkreader::{DARKREADER_DISABLE_SCRIPT, DARKREADER_ENABLE_SCRIPT},
    error::FrameworkError,
    history::{HistoryEvent, HistorySnapshotEntry},
    state::BrowserState,
    tab_service::{on_document_title_changed, on_download, on_new_window, on_page_load},
    user_agent::get_user_agent,
};

const BLANK_URL: &str = "about:blank";

/// 窗口/webview 背景色（跟随系统主题）
pub(crate) fn bg_color(is_dark: bool) -> Color {
    if is_dark {
        Color(29, 35, 42, 255)
    } else {
        Color(255, 255, 255, 255)
    }
}

pub type TabId = String;

#[derive(Debug, Clone)]
struct HistoryEntry {
    id: i64,
    url: String,
    /// Navigation API 上报的条目身份（会话内唯一稳定）；降级路径下为 None
    key: Option<String>,
}

pub struct Tab {
    webview: Webview,
    title: String,
    icon_url: String,
    loading: bool,
    /// 等待中的同文档导航（pushState/popstate）或 bfcache 恢复：
    /// 不触发页面加载事件，需靠 Navigation API 快照确认导航完成并清理 loading
    nav_pending: bool,
    /// Finished 已触发但 LoadFinished 历史校准尚未消费：跨文档加载完成瞬间的
    /// 标题变更应 defer（pending_title）而非直接落库，避免与校准重复写入
    load_finished_pending: bool,
    /// 加载中或 LoadFinished 校准在途时到达、被 defer 落库的标题：
    /// 待校准（on_page_load_finished_history）或快照对账（sync_snapshot）后
    /// 用权威 URL 补写，避免标题写进旧 URL 记录/同一行重复落库
    pending_title: Option<String>,
    /// 导航发起时乐观设置的 URL（按 tab 隔离），在真实导航追上之前，
    /// get_state 会使用此值覆盖 URL 与 loading，避免网络延迟时无反应。
    optimistic_url: Option<String>,
    /// on_page_load(Started) 时的乐观 URL 快照：Finished 时若快照与当前乐观 URL
    /// 不一致，说明期间有新导航上报，Finished 属于旧导航，不得误清新导航的乐观 URL
    start_optimistic_url: Option<String>,
    darkreader: bool,
    index: isize,
    /// 当前加载是否为重定向（reload / 302 / meta refresh）：
    /// 浏览器对重定向是替换当前条目而非新增，sync 时据此选择语义
    redirecting: bool,
    /// 该 tab（webview）独立的历史事件 FIFO 队列：
    /// 同 tab 内按入队顺序应用，不同 tab 互不阻塞；
    /// 随 Tab 创建而启动专属消费者，随 Tab 销毁而关闭通道、退出消费者
    history_queue: async_runtime::Sender<HistoryEvent>,
    history: Vec<HistoryEntry>,
}

impl Deref for Tab {
    type Target = Webview;

    fn deref(&self) -> &Self::Target {
        &self.webview
    }
}

impl Tab {
    pub fn new(
        window: &Window,
        label: &str,
        url: &Url,
        incognito: bool,
        history_queue: async_runtime::Sender<HistoryEvent>,
    ) -> Result<Self, FrameworkError> {
        let mut size = window
            .inner_size()?
            .to_logical::<f64>(window.scale_factor()?);
        // 窗口可能启动即处于全屏（恢复窗口状态），此时没有标题栏
        let is_fullscreen = window.is_fullscreen()?;
        let position = if is_fullscreen {
            LogicalPosition::new(0., 0.)
        } else {
            size.height -= Webview::TITLE_HEIGHT;
            LogicalPosition::new(0., Webview::TITLE_HEIGHT)
        };

        let app_handle = window.app_handle().clone();
        let is_dark = matches!(window.theme()?, Theme::Dark);
        let builder = WebviewBuilder::new(label, WebviewUrl::External(url.clone()))
            .initialization_script(include_str!("../js/darkreader.js"))
            .initialization_script(include_str!("../js/webview_init.js"))
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
            load_finished_pending: false,
            pending_title: None,
            optimistic_url: None,
            start_optimistic_url: None,
            darkreader: true,
            redirecting: false,
            history_queue,
            history: Vec::new(),
            index: -1,
        })
    }

    pub fn index(&self, id: i64) -> Option<usize> {
        self.history.iter().position(|item| item.id == id)
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
    pub fn current_url(&self) -> Result<String, Error> {
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

        // 用 History API（非 Navigation API）：navigation.back() 对跨文档导航
        // "永不应决"（WICG/WebKit 规范），history.back() 跨源、跨文档均可用
        self.webview
            .eval("history.back()")
            .inspect_err(|e| error!("{}后退失败{e}", self.label()))
            .is_ok()
    }

    pub fn forward(&mut self) -> bool {
        if !self.can_forward() {
            return false;
        }

        self.webview
            .eval("history.forward()")
            .inspect_err(|e| error!("{}前进失败{e}", self.label()))
            .is_ok()
    }

    pub fn go(&mut self, index: usize) -> bool {
        let index = index as isize;
        if self.index == index {
            return false;
        }

        // 相对 delta 而非绝对 index：镜像 index 由快照对账实时校准，与浏览器
        // 会话历史严格一致，delta 不会漂移
        let script = format!("history.go({})", index - self.index);
        self.webview
            .eval(script)
            .inspect_err(|e| error!("{}跳转失败{e}", self.label()))
            .is_ok()
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

        // 当前 index 指向的条目已是该 URL：镜像已同步（快照对账/此前 sync 均按
        // 真实导航序落好），直接返回。不能在 URL 重复时按"最近匹配"重定位 index：
        // 跨文档导航中快照已把新 URL 插入 index+1，迟到的校准（LoadFinished 的
        // state.url 取自镜像、可能仍是旧文档）会命中 index 处的旧条目而把 index
        // 拉回上一页面，表现为"点击链接后历史被重置"
        let cur = self.index.max(0) as usize;
        let cur_entry = self.history.get(cur);
        if cur_entry.is_some_and(|e| e.url == url)
            || cur_entry.is_some_and(|e| {
                let url_trimmed = url.trim_end_matches('/');
                !url_trimmed.is_empty() && e.url.trim_end_matches('/') == url_trimmed
            })
        {
            return false; // 当前条目已匹配，未插入新条目
        }

        // 查找 URL 是否已存在于历史栈：精确匹配优先，其次容忍尾部斜杠差异；
        // 有多个匹配时选择距离当前 index 最近的位置（最小移动量），
        // 避免 position() 固定选第一个匹配而在 URL 重复时定位错误
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

    /// 以 Navigation API 上报的快照对账历史镜像，核心逻辑见
    /// [`Self::reconcile_snapshot`]。
    pub fn sync_snapshot(
        &mut self,
        index: usize,
        entries: Vec<HistorySnapshotEntry>,
    ) -> (Vec<(usize, String)>, usize) {
        let Self {
            history,
            index: mirror_index,
            redirecting,
            ..
        } = self;
        Self::reconcile_snapshot(history, mirror_index, redirecting, index, entries)
    }

    /// 条目同一性（用于非对齐锚定窗口核对）：双方均有 key 时必须 key 相等
    /// （会话内唯一身份）——同 URL 的两次访问是不同 key 的不同条目，不得
    /// 并为一条；仅镜像侧 keyless 占位条目按 URL 认领（容忍尾部斜杠差异，
    /// 新 tab 首个快照原地补 key 的场景）
    fn same_entry(m: &HistoryEntry, f: &HistoryEntry) -> bool {
        if m.key.is_some() {
            m.key == f.key
        } else {
            m.url.trim_end_matches('/') == f.url.trim_end_matches('/')
        }
    }

    /// 原位采纳快照条目：key/URL 以快照为准。URL 未变且镜像 id 有效时保留
    /// 原 id（keyless 占位原地补 key 的场景），避免已落库条目被置占位后重复
    /// 落库；URL 已变（replaceState）才置占位等待回填
    fn adopt_entry(m: &mut HistoryEntry, f: &HistoryEntry) {
        if m.id <= 0 || f.id > 0 || m.url.trim_end_matches('/') != f.url.trim_end_matches('/') {
            m.id = f.id;
        }
        m.url = f.url.clone();
        m.key = f.key.clone();
    }

    /// 在镜像中按 key 顺序定位片段全部条目（严格递增子序列匹配）；
    /// 任一条目 key 未命中则返回 None。用于同源片段不连续（条目间夹异源
    /// 历史，entries() 过滤后未必连续）时的纯状态同步
    fn locate_fragment_subsequence(
        history: &[HistoryEntry],
        fragment: &[HistoryEntry],
    ) -> Option<Vec<usize>> {
        if fragment.is_empty() {
            return None;
        }
        let mut positions = Vec::with_capacity(fragment.len());
        let mut from = 0usize;
        for f in fragment {
            let offset = history[from..]
                .iter()
                .position(|m| m.key.is_some() && m.key == f.key)?;
            positions.push(from + offset);
            from += offset + 1;
        }
        Some(positions)
    }

    /// 以 Navigation API 上报的快照对账历史镜像（不依赖 webview 的纯函数）。
    /// **快照只是"与当前源同源的连续片段"**（Navigation API 安全限制）：
    /// 跨源导航后 entries() 不含异源历史，`snapshot_index` 是该片段内的位置
    /// 而非联合历史的全局位置——绝不能全量重建，否则跨源跳转后历史被抹成
    /// 只剩片段（"点击链接跳转页面后，Tab 历史重置了"的根因）。
    ///
    /// 对账规则：
    /// - 首条目 key 命中镜像已有 key 且 key 序列对齐 → 以快照替换从该位置起
    ///   的镜像历史，修正 index 为片段内当前位置；
    /// - key 序列不对齐（后退返回祖先条目后又有新导航 / replaceState 换 key /
    ///   跨源片段）→ 先尝试子序列定位（片段全部 key 在镜像中按序命中，覆盖
    ///   同源片段不连续的场景），否则将片段锚定到以镜像当前条目或其祖先结尾
    ///   的窗口（候选遍历 0..=片段当前位置），逐位核对 key/URL 后原位更新
    ///   已有条目、只追加真正新增的尾部——盲目整段追加会把片段与镜像的共同
    ///   前缀重复插入，并把 index 推到片段末尾（"后退成功但标题栏 URL 不更新、
    ///   不能前进"的根因）；
    /// - 子序列与锚定窗口均未命中（跨源片段首条即新 URL）→ 截断当前位置
    ///   之后的 forward 历史后整段追加。
    ///
    /// key 作为条目身份：同 key 且 URL 未变的条目保留原 id；新 key 或同 key
    /// 但 URL 已变（replaceState 只改 URL 不改 key）置占位 -1 等待回填。
    /// 占位回填只覆盖**本次快照影响的片段区间**，不重复落库镜像中历史遗留的
    /// 占位条目（save_log 对已存在行 title 非空且 icon 到位时 times + 1，重复会
    /// 虚增计数）。
    /// 返回 (需要回填 id 的 (位置, url) 列表, 对账后当前条目在镜像中的位置)。
    fn reconcile_snapshot(
        history: &mut Vec<HistoryEntry>,
        index: &mut isize,
        redirecting: &mut bool,
        snapshot_index: usize,
        entries: Vec<HistorySnapshotEntry>,
    ) -> (Vec<(usize, String)>, usize) {
        // key 作为条目身份：同 key 且 URL 未变的条目保留原 id，
        // 新 key 或 URL 变更（replaceState）置占位 -1 等待回填
        let id_by_key: StdHashMap<&str, (i64, &str)> = history
            .iter()
            .filter_map(|entry| {
                entry
                    .key
                    .as_deref()
                    .map(|key| (key, (entry.id, entry.url.as_str())))
            })
            .collect();

        let fragment: Vec<HistoryEntry> = entries
            .iter()
            .map(|entry| HistoryEntry {
                id: match id_by_key.get(entry.key.as_str()) {
                    Some((id, url)) if *url == entry.url => *id,
                    _ => -1,
                },
                url: entry.url.clone(),
                key: Some(entry.key.clone()),
            })
            .collect();
        let frag_len = fragment.len();
        // 片段内首条目的镜像位置；命中 = 片段是镜像已有子段，未命中 = 新片段
        let frag_start = match fragment.first().and_then(|e| e.key.as_deref()) {
            Some(key) => history
                .iter()
                .position(|entry| entry.key.as_deref() == Some(key)),
            None => None,
        };
        // 校验 key 序列对齐，防范跨源场景（Navigation API 跨源时不返回异源条目，
        // 导致片段是镜像子集，splice 会错误替换中间条目）。
        // 对齐规则：忽略镜像中 keyless 占位条目（sync_by_url 先插入的占位），
        // 有 key 的条目必须与片段逐一对齐，否则视为跨源片段回退到追加分支
        let aligned = frag_start.is_some_and(|start| {
            history[start..].iter().zip(fragment.iter()).all(|(m, f)| {
                // keyless 占位条目不参与对齐检查（sync_by_url 先插入的占位
                // 无 key，导致 aligned 误判为 false，回退到追加分支反复追加）
                m.key.is_none() || m.key.as_deref() == f.key.as_deref()
            })
        });
        // (对账后当前 index, 本次快照影响的镜像区间 [start, end)，用于限定回填范围)
        let (cur, span) = if let Some(start) = frag_start
            && aligned
        {
            // 片段首条 key 命中镜像已有条目且 key 序列对齐：以快照替换从 start
            // 位置起的镜像历史。使用 end = min(...) 安全处理片段超出历史长度的
            // 情况（keyless 占位条目导致历史较短）。
            let end = (start + frag_len).min(history.len());
            history.splice(start..end, fragment);
            ((start + snapshot_index) as isize, start..start + frag_len)
        } else {
            // key 序列不对齐：先尝试子序列定位（纯状态同步），失败再锚定合并。
            // 防御 index 为 -1（空历史新 tab）导致的下溢
            let at = (*index).max(0) as usize;

            // ① 子序列定位：片段所有条目的 key 都能在镜像中按序找到（同源
            // 片段不连续，条目间夹着快照看不到的异源历史，如 a.com → x.com
            // → a.com）→ 结构不变，原位更新命中条目、index 重定位到片段
            // 当前条目。绝不整段追加：共同条目重复插入正是"后退后 URL 不
            // 更新、历史膨胀"的根因形态
            if let Some(positions) = Self::locate_fragment_subsequence(history, &fragment) {
                for (pos, f) in positions.iter().zip(&fragment) {
                    Self::adopt_entry(&mut history[*pos], f);
                }
                let first = positions[0];
                let last = positions[positions.len() - 1];
                let cur = positions[snapshot_index.min(positions.len() - 1)] as isize;
                *index = cur;
                (cur, first..last + 1)
            } else {
                // ② 锚定合并：把片段锚定到以镜像当前条目（或其某一祖先，候选
                // 锚点遍历 0..=片段当前条目位置，覆盖多次新导航合并进同一快照
                // 的场景）结尾的窗口上
                let anchored = (0..=snapshot_index)
                    .filter_map(|offset| at.checked_sub(offset))
                    .map(|start| {
                        let mut matched = 0usize;
                        while matched < frag_len
                            && start + matched < history.len()
                            && Self::same_entry(&history[start + matched], &fragment[matched])
                        {
                            matched += 1;
                        }
                        (start, matched)
                    })
                    .max_by_key(|&(_, matched)| matched);
                match anchored.filter(|&(_, matched)| matched > 0) {
                    Some((start, matched)) => {
                        // 已核对区间原位更新；keyless 占位按 URL 认领补 key
                        for j in 0..matched {
                            Self::adopt_entry(&mut history[start + j], &fragment[j]);
                        }
                        // 锚点窗口内核对不上的尾部条目已被新导航替换：截断后
                        // 追加片段剩余部分（含片段当前条目与 forward 历史）
                        history.truncate(start + matched);
                        history.extend(fragment[matched..].iter().cloned());
                        // 片段在镜像中占位连续：fragment[i] 落在 start + i
                        *index = (start + snapshot_index).min(history.len() - 1) as isize;
                        (*index, start..history.len())
                    }
                    None => {
                        // 子序列与锚定窗口均未命中（如跨源片段首条即新 URL）：
                        // 截断当前位置之后的 forward 历史，整段追加。
                        // index 不能取"末尾"：无共同条目的片段可能带有 forward
                        // 条目（snapshot_index < frag_len - 1），片段当前条目
                        // 落在 append_start + snapshot_index 处
                        if at != history.len() {
                            history.truncate(at + 1);
                        }
                        let append_start = history.len();
                        history.extend(fragment);
                        *index = (append_start + snapshot_index)
                            .min(history.len().saturating_sub(1))
                            as isize;
                        (*index, append_start..history.len())
                    }
                }
            }
        };
        *index = cur;
        *redirecting = false;

        // 占位条目（本次快照影响的片段内新 key / replaceState 改 URL）等待回填。
        // 只收集影响区间内的条目，避免每次快照重复落库镜像中历史遗留的占位条目
        // （save_log 对已存在行 title 非空且 icon 到位时 times + 1，会虚增访问计数）
        let needs_id = history[span.clone()]
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.id <= 0)
            .map(|(offset, entry)| (span.start + offset, entry.url.clone()))
            .collect();
        (needs_id, cur as usize)
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

    pub fn set_darkreader(&mut self, enable: bool) -> Result<(), Error> {
        self.eval(if enable {
            DARKREADER_ENABLE_SCRIPT
        } else {
            DARKREADER_DISABLE_SCRIPT
        })
        .inspect(|_| self.darkreader = enable)
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
        if let Some((_, tab)) = self.0.remove_async(label).await {
            tab.close()?;
        }
        Ok(())
    }

    pub async fn contains(&self, label: &str) -> bool {
        self.0.contains_sync(label)
    }

    /// 返回所有 label（供 close_incognito 遍历使用）
    pub async fn keys(&self) -> Vec<TabId> {
        let mut labels = Vec::new();
        self.0
            .iter_async(|l, _| {
                labels.push(l.to_owned());
                true
            })
            .await;
        labels
    }

    /// 返回 id 所在的 (label, index)
    pub async fn any_open(&self, id: i64) -> Option<(TabId, usize)> {
        let mut label = None;
        self.0
            .any_async(|l, tab| {
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
            .unwrap_or(Err(Error::WebviewNotFound))?;
        Ok(())
    }

    pub async fn set_focus(&self, label: &str) -> Result<(), FrameworkError> {
        self.0
            .read_async(label, |_, tab| tab.set_focus())
            .await
            .unwrap_or(Err(Error::WebviewNotFound))?;
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

    /// 记录加载中或校准在途时被 defer 的标题，待校准/快照对账后用权威 URL 补写
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

    pub async fn set_optimistic_url(&self, label: &str, url: String) {
        self.0
            .update_async(label, |_, tab| tab.optimistic_url = Some(url))
            .await;
    }

    /// 安全清除乐观 URL：乐观 URL 与 on_page_load(Started) 时的快照不一致，
    /// 说明本次 Finished 属于旧导航，不得误清新导航设置的乐观 URL
    pub async fn try_clear_optimistic_url(&self, label: &str) {
        self.0
            .update_async(label, |_, tab| {
                if tab.optimistic_url == tab.start_optimistic_url {
                    tab.optimistic_url = None
                }
            })
            .await;
    }

    pub async fn clear_optimistic_url(&self, label: &str) {
        self.0
            .update_async(label, |_, tab| tab.optimistic_url = None)
            .await;
    }

    /// 快照当前乐观 URL，用于 on_page_load(Finished) 检测 stale 事件
    pub async fn snapshot_optimistic_url(&self, label: &str) {
        self.0
            .update_async(label, |_, tab| {
                tab.start_optimistic_url = tab.optimistic_url.clone()
            })
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

    /// 读取该 tab 的历史事件队列发送端；tab 已关闭时为 None
    pub async fn history_queue(&self, label: &str) -> Option<async_runtime::Sender<HistoryEvent>> {
        self.0
            .read_async(label, |_, tab| tab.history_queue.clone())
            .await
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

    /// 标记 LoadFinished 历史校准是否在途（Finished 已触发、校准事件尚未消费）：
    /// 该窗口内的标题变更应 defer 到校准落库，避免与校准重复写入同一行
    pub async fn set_load_finished_pending(&self, label: &str, pending: bool) {
        self.0
            .update_async(label, |_, tab| tab.load_finished_pending = pending)
            .await;
    }

    pub async fn is_load_finished_pending(&self, label: &str) -> bool {
        self.0
            .read_async(label, |_, tab| tab.load_finished_pending)
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

    /// 片段对账：以 Navigation API 快照对账镜像（快照只是同源连续片段，
    /// 不是联合历史全量），返回 (需要回填 id 的 (位置, url) 列表, 对账后当前位置)
    pub async fn sync_snapshot(
        &self,
        label: &str,
        index: usize,
        entries: Vec<HistorySnapshotEntry>,
    ) -> (Vec<(usize, String)>, usize) {
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

    pub async fn set_darkreader(&self, label: &str, enable: bool) -> Result<(), Error> {
        self.0
            .update_async(label, |_, tab| tab.set_darkreader(enable))
            .await
            .unwrap_or(Ok(()))
    }

    pub async fn set_background_color(&self, label: &str, color: Color) -> Result<(), Error> {
        self.0
            .update_async(label, |_, tab| tab.set_background_color(Some(color)))
            .await
            .unwrap_or(Ok(()))
    }

    pub async fn darkreader(&self, label: &str) -> Result<bool, Error> {
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
            .unwrap_or(Err(Error::WebviewNotFound))?;
        Ok(())
    }

    pub async fn get_state(&self, label: &str) -> Result<BrowserState, FrameworkError> {
        let state = self
            .0
            .read_async(label, |_, tab| {
                let url = tab.current_url()?;
                let loading = tab.loading;
                // 乐观 URL 覆盖：真实 URL 尚未追上时，用 navigate_started 设置的 URL 替代
                let (url, loading) = if let Some(ref opt_url) = tab.optimistic_url
                    && url != *opt_url
                {
                    (opt_url.clone(), true)
                } else {
                    (url, loading)
                };
                Ok(BrowserState {
                    icon_url: tab.icon_url.clone(),
                    title: tab.title.clone(),
                    url,
                    loading,
                    can_back: tab.can_back(),
                    can_forward: tab.can_forward(),
                    darkreader: tab.darkreader,
                    ..Default::default()
                })
            })
            .await
            .unwrap_or(Err(Error::WebviewNotFound))?;

        Ok(state)
    }

    pub async fn next(&self, label: &str) -> Option<TabId> {
        let labels = self.keys().await;
        if labels.len() <= 1 {
            return None;
        }
        labels
            .iter()
            .filter(|l| l.as_str() < label)
            .max()
            .cloned()
            .or_else(|| labels.iter().max().cloned())
    }

    pub async fn near(&self, label: &str) -> Option<TabId> {
        let labels = self.keys().await;
        if labels.len() <= 1 {
            return None;
        }
        if let Some(next) = labels.iter().filter(|l| l.as_str() > label).min().cloned() {
            return Some(next);
        }
        self.next(label).await
    }
}

#[cfg(test)]
mod tests {
    use super::{HistoryEntry, HistorySnapshotEntry, Tab};

    fn entry(id: i64, url: &str, key: Option<&str>) -> HistoryEntry {
        HistoryEntry {
            id,
            url: url.to_string(),
            key: key.map(str::to_string),
        }
    }

    fn snap(key: &str, url: &str) -> HistorySnapshotEntry {
        HistorySnapshotEntry {
            key: key.to_string(),
            url: url.to_string(),
        }
    }

    fn assert_entry(history: &[HistoryEntry], pos: usize, id: i64, url: &str) {
        assert_eq!(history[pos].id, id, "位置 {pos} 的 id 不符");
        assert_eq!(history[pos].url, url, "位置 {pos} 的 url 不符");
    }

    /// 回归："后退 → 浏览新页面 → 再后退"后标题栏 URL 不更新、不能前进。
    /// 修复前：浏览 C 时非对齐分支整段追加，把片段与镜像的共同前缀 A 重复
    /// 插入（镜像变 [A,A,C]），再次后退时对账失败、index 停在 C。
    /// 修复后：锚定合并只追加新增尾部，镜像无重复、index 正确。
    #[test]
    fn back_after_new_navigation_keeps_mirror_consistent() {
        let mut history: Vec<HistoryEntry> = Vec::new();
        let mut index = 0isize;
        let mut redirecting = false;

        // 新 tab 打开 A：keyless 占位 + 首个快照原地补 key，id 保留
        history.push(entry(1, "https://a.com/", None));
        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            0,
            vec![snap("k1", "https://a.com/")],
        );
        assert_eq!(cur, 0);
        assert!(needs.is_empty());
        assert_entry(&history, 0, 1, "https://a.com/");

        // 浏览 B（同源跨文档）：对齐替换，B 为占位待回填
        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            1,
            vec![snap("k1", "https://a.com/"), snap("k2", "https://a.com/b")],
        );
        assert_eq!(cur, 1);
        assert_eq!(needs, vec![(1, "https://a.com/b".to_string())]);
        // 模拟回填
        history[1].id = 2;

        // 后退到 A：对齐替换，index 修正为 0
        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            0,
            vec![snap("k1", "https://a.com/"), snap("k2", "https://a.com/b")],
        );
        assert_eq!(cur, 0);
        assert!(needs.is_empty());

        // 浏览 C（同源、新 key）：片段 [A,C] 与镜像 [A,B] key 不对齐，
        // 锚定合并应只追加 C，不得重复插入 A
        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            1,
            vec![snap("k1", "https://a.com/"), snap("k3", "https://a.com/c")],
        );
        assert_eq!(cur, 1);
        assert_eq!(needs, vec![(1, "https://a.com/c".to_string())]);
        history[1].id = 3;
        assert_eq!(history.len(), 2, "不得产生重复条目");
        assert_entry(&history, 0, 1, "https://a.com/");
        assert_entry(&history, 1, 3, "https://a.com/c");

        // 再后退到 A：这次对账必须成功——index 回 0、URL 为 A、可前进
        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            0,
            vec![snap("k1", "https://a.com/"), snap("k3", "https://a.com/c")],
        );
        assert_eq!(cur, 0, "后退后 index 应回到 A");
        assert!(needs.is_empty());
        assert_eq!(history.len(), 2, "后退后镜像不得膨胀或重复");
        assert_eq!(index, 0, "A 是历史首条目，后退后 index 应为 0");
        assert!(
            index < history.len() as isize - 1,
            "C 仍在 forward 历史中，应可前进"
        );
    }

    /// 跨源片段不回归：异源条目不在快照里，锚定/追加均不得抹掉镜像中
    /// 的异源历史（"点击链接跳转页面后，Tab 历史重置了"的既有根因）
    #[test]
    fn cross_origin_fragment_preserves_out_of_fragment_history() {
        let mut history: Vec<HistoryEntry> = Vec::new();
        let mut redirecting = false;

        history.push(entry(1, "https://a.com/", Some("k1")));
        history.push(entry(2, "https://x.com/", Some("kx")));
        let mut index = 1isize;

        // 从 x.com 后退到 a.com：快照只含同源片段 [A]，index 为片段内位置 0
        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            0,
            vec![snap("k1", "https://a.com/")],
        );
        assert_eq!(cur, 0);
        assert!(needs.is_empty());
        assert_eq!(history.len(), 2, "异源 forward 条目不得被抹掉");
        assert_entry(&history, 0, 1, "https://a.com/");
        assert_entry(&history, 1, 2, "https://x.com/");
        assert!(
            index < history.len() as isize - 1,
            "异源 forward 条目应可前进"
        );
    }

    /// 跨源新导航：片段与镜像无任何共同条目，走整段追加分支，
    /// index 移到片段末尾（行为与修复前一致）
    #[test]
    fn disjoint_cross_origin_fragment_falls_back_to_append() {
        let mut history: Vec<HistoryEntry> = Vec::new();
        let mut redirecting = false;

        history.push(entry(1, "https://a.com/", Some("k1")));
        let mut index = 0isize;

        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            0,
            vec![snap("kc", "https://c.com/")],
        );
        assert_eq!(cur, 1, "index 移到追加后的片段末尾");
        assert_eq!(needs, vec![(1, "https://c.com/".to_string())]);
        assert_eq!(history.len(), 2, "追加分支保留镜像当前条目再追加片段");
        assert_entry(&history, 0, 1, "https://a.com/");
        assert_entry(&history, 1, -1, "https://c.com/");
        assert_eq!(index, 1);
    }

    /// replaceState 换 URL（key 不变）：对齐分支原位替换并置占位等待回填
    #[test]
    fn replace_state_updates_url_and_marks_placeholder() {
        let mut history: Vec<HistoryEntry> = Vec::new();
        history.push(entry(1, "https://a.com/", Some("k1")));
        history.push(entry(2, "https://a.com/b", Some("k2")));
        let mut index = 1isize;
        let mut redirecting = false;

        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            1,
            vec![
                snap("k1", "https://a.com/"),
                snap("k2", "https://a.com/b?q=1"),
            ],
        );
        assert_eq!(cur, 1);
        assert_eq!(needs, vec![(1, "https://a.com/b?q=1".to_string())]);
        assert_entry(&history, 0, 1, "https://a.com/");
        assert_eq!(history[1].id, -1, "URL 已变的条目应置占位");
        assert_eq!(history[1].url, "https://a.com/b?q=1");
    }

    /// 空历史新 tab（index = -1）的首个快照：不得下溢，正常建条
    #[test]
    fn first_snapshot_on_empty_history() {
        let mut history: Vec<HistoryEntry> = Vec::new();
        let mut index: isize = -1;
        let mut redirecting = false;

        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            0,
            vec![snap("k1", "https://a.com/")],
        );
        assert_eq!(cur, 0);
        assert_eq!(needs, vec![(0, "https://a.com/".to_string())]);
        assert_eq!(history.len(), 1);
        assert_entry(&history, 0, -1, "https://a.com/");
        assert_eq!(index, 0);
    }

    /// 同一 URL 的两次访问是不同 key 的不同条目：核对时不得按 URL 并为
    /// 一条（URL 兜底仅限 keyless 占位认领），否则两次访问会折叠、key 被
    /// 覆写导致后续对齐漂移
    #[test]
    fn same_url_with_distinct_keys_stays_distinct() {
        let keyed_first = entry(1, "https://a.com/", Some("k1"));
        let keyed_second = entry(3, "https://a.com/", Some("k3"));
        let keyless = entry(-1, "https://a.com/", None);

        assert!(
            Tab::same_entry(&keyed_first, &keyed_first),
            "同 key 同一条目"
        );
        assert!(
            !Tab::same_entry(&keyed_first, &keyed_second),
            "同 URL 不同 key 是不同访问，不得视为同一条目"
        );
        assert!(
            Tab::same_entry(&keyless, &keyed_second),
            "keyless 占位条目按 URL 认领（新 tab 首个快照补 key）"
        );
    }

    /// 非连续同源片段：条目间夹着快照看不到的异源历史（a.com → x.com →
    /// a.com）时，子序列定位应纯状态同步——index 重定位、不重复插入共同
    /// 条目、不抹掉夹在中间的异源条目
    #[test]
    fn non_contiguous_fragment_relocates_without_duplication() {
        let mut history: Vec<HistoryEntry> = Vec::new();
        let mut redirecting = false;

        history.push(entry(1, "https://a.com/", Some("k1")));
        history.push(entry(2, "https://x.com/", Some("kx")));
        history.push(entry(3, "https://a.com/b", Some("k2")));
        let mut index = 2isize;

        // 从 a.com/b 后退到 a.com：快照为同源但不连续的 [A, B]
        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            0,
            vec![snap("k1", "https://a.com/"), snap("k2", "https://a.com/b")],
        );
        assert_eq!(cur, 0, "后退后 index 重定位到片段当前条目 A");
        assert!(needs.is_empty(), "命中条目 id 均有效，无需回填");
        assert_eq!(history.len(), 3, "不得重复插入 A/B，不得抹掉异源 X");
        assert_entry(&history, 0, 1, "https://a.com/");
        assert_entry(&history, 1, 2, "https://x.com/");
        assert_entry(&history, 2, 3, "https://a.com/b");
        assert!(index < history.len() as isize - 1, "后退保留 forward 历史");
    }

    /// 后退后再前进：对齐分支把 index 推回片段当前条目，forward 恢复
    #[test]
    fn forward_after_back_restores_position() {
        let mut history: Vec<HistoryEntry> = Vec::new();
        let mut redirecting = false;

        history.push(entry(1, "https://a.com/", Some("k1")));
        history.push(entry(2, "https://a.com/c", Some("k3")));
        let mut index = 0isize;

        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            1,
            vec![snap("k1", "https://a.com/"), snap("k3", "https://a.com/c")],
        );
        assert_eq!(cur, 1, "前进后 index 指向 C");
        assert!(needs.is_empty(), "key + URL 均未变，id 保留无需回填");
        assert_eq!(history.len(), 2);
        assert_entry(&history, 1, 2, "https://a.com/c");
        assert_eq!(index, 1);
    }

    /// 同 URL 重访：后退到 A 后再次导航到 C（新 key、旧 C 条目已被会话
    /// 截断）——不得按 URL 兜底把新访问并进旧条目，新条目置占位等待回填
    #[test]
    fn revisit_same_url_creates_new_entry() {
        let mut history: Vec<HistoryEntry> = Vec::new();
        let mut redirecting = false;

        history.push(entry(1, "https://a.com/", Some("k1")));
        history.push(entry(3, "https://a.com/c", Some("k3")));
        let mut index = 0isize;

        // 从 A 再次导航到 C：片段 [A(k1), C(k4)]，镜像 [A(k1), C(k3)]
        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            1,
            vec![snap("k1", "https://a.com/"), snap("k4", "https://a.com/c")],
        );
        assert_eq!(cur, 1);
        assert_eq!(
            needs,
            vec![(1, "https://a.com/c".to_string())],
            "新 key 条目应置占位待回填"
        );
        assert_eq!(history.len(), 2, "不得重复条目");
        assert_entry(&history, 0, 1, "https://a.com/");
        assert_eq!(
            history[1].key.as_deref(),
            Some("k4"),
            "新访问的 key 以快照为准"
        );
        assert_eq!(history[1].id, -1, "新访问是新记录，不得复用旧 id");
    }

    /// 多次新导航合并进同一快照（片段当前条目距镜像当前条目隔一个祖先）：
    /// 锚点候选遍历 0..=snapshot_index，深层祖先窗口也能命中
    #[test]
    fn multi_navigation_snapshot_anchors_at_deeper_ancestor() {
        let mut history: Vec<HistoryEntry> = Vec::new();
        let mut redirecting = false;

        history.push(entry(1, "https://a.com/", Some("k1")));
        history.push(entry(2, "https://x.com/", Some("kx")));
        let mut index = 0isize;

        // 镜像在 A（后退态），期间连续导航 B、C 只上报最终快照 [A,B,C]，
        // index 为片段内位置 2；异源 X 应被新导航的 forward 截断替换
        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            2,
            vec![
                snap("k1", "https://a.com/"),
                snap("k2", "https://a.com/b"),
                snap("k3", "https://a.com/c"),
            ],
        );
        assert_eq!(cur, 2, "index 指向片段当前条目 C");
        assert_eq!(
            needs,
            vec![
                (1, "https://a.com/b".to_string()),
                (2, "https://a.com/c".to_string())
            ]
        );
        assert_eq!(history.len(), 3, "X 被截断，追加 B、C");
        assert_entry(&history, 0, 1, "https://a.com/");
        assert_entry(&history, 1, -1, "https://a.com/b");
        assert_entry(&history, 2, -1, "https://a.com/c");
        assert_eq!(index, 2);
    }

    /// 无共同条目的片段带有 forward 条目（snapshot_index < frag_len - 1）：
    /// 兜底追加后 index 落在 append_start + snapshot_index，而非片段末尾
    #[test]
    fn disjoint_fragment_with_forward_entries_keeps_current_position() {
        let mut history: Vec<HistoryEntry> = Vec::new();
        let mut redirecting = false;
        let mut index: isize = -1;

        // 镜像为空，片段为异源 [C, D] 且当前条目是 C（index 0，D 在 forward）
        let (needs, cur) = Tab::reconcile_snapshot(
            &mut history,
            &mut index,
            &mut redirecting,
            0,
            vec![snap("kc", "https://c.com/"), snap("kd", "https://c.com/d")],
        );
        assert_eq!(cur, 0, "当前条目 C 在追加后应位于 0，而非片段末尾");
        assert_eq!(needs.len(), 2, "两条均为待回填占位");
        assert_eq!(history.len(), 2);
        assert_entry(&history, 0, -1, "https://c.com/");
        assert_entry(&history, 1, -1, "https://c.com/d");
        assert_eq!(index, 0);
        assert!(index < history.len() as isize - 1, "D 在 forward 历史中");
    }
}
