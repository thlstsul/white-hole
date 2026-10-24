use std::{error::Error as StdError, fmt};

use delay_timer::error::TaskError;
use error_set::error_set;
use get_data_url::Error as GetDataUrlError;
use publicsuffix::Error as PublicSuffixError;
use reqwest::Error as ReqwestError;
use serde_json::Error as SerdeJsonError;
use sqlx::Error as SqlxError;
use sqlx::migrate::MigrateError;
use tauri::Error as TauriError;
use time::error::IndeterminateOffset;
use url::ParseError as UrlParseError;

use crate::impl_serialize;

/// Tab 未找到错误，包装 String 使其实现 std::error::Error
#[derive(Debug)]
pub struct TabNotFoundError(pub String);

impl fmt::Display for TabNotFoundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl StdError for TabNotFoundError {}

error_set! {
    ParseError := {
        #[display("无法解析的URL: {0}")]
        Url(UrlParseError),
        #[display("无法解析的公共后缀: {0}")]
        PublicSuffix(PublicSuffixError),
    }
    FrameworkError := {
        #[display("Tauri 框架错误: {0}")]
        Tauri(TauriError),
    }
    DatabaseError := {
        #[display("数据库执行错误: {0}")]
        Execute(SqlxError),
        #[display("数据库迁移错误: {0}")]
        Migrate(MigrateError),
    }
    SetupError := {
        #[display("后台进程设置错误: {0}")]
        Task(TaskError),
    } || DatabaseError || FrameworkError || ParseError
    TabError := StateError || FrameworkError || ParseError
    StateError := {
        #[display("Tab 未找到: {0}")]
        TabNotFound(TabNotFoundError),
    } || FrameworkError || DatabaseError || IconError
    IconError := {
        #[display("无法获取图标数据：{0}")]
        GetDataUrl(GetDataUrlError),
        #[display("无法保存图标：{0}")]
        SaveIcon(SqlxError),
        #[display("图标获取中")]
        Fetching,
    }
    FetchError := {
        #[display("网络请求失败：{0}")]
        Fetch(ReqwestError),
        #[display("请求体不是合法的 JSON：{0}")]
        InvalidJson(SerdeJsonError),
        #[display("计算耗时失败：{0}")]
        Time(IndeterminateOffset),
    }
    SyncPublicSuffixError := FetchError || DatabaseError
    GetPublicSuffixError := ParseError || DatabaseError
}

impl_serialize![
    ParseError,
    DatabaseError,
    FrameworkError,
    TabError,
    StateError,
    FetchError
];
