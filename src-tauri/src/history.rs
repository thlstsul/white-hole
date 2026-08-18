/// 历史事件：同一 tab 内的历史写入统一经该 tab 独立的 FIFO 队列
/// 由单消费者按序应用，避免并发 IPC 命令乱序应用镜像；
/// 不同 tab（webview）各自持有独立队列，互不阻塞。
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

/// Navigation API 快照中的一条历史条目（key 作为条目身份）
#[derive(Debug, Clone, serde::Deserialize)]
pub struct HistorySnapshotEntry {
    pub key: String,
    pub url: String,
}
