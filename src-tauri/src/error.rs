use error_set::error_set;

use crate::impl_serialize;

error_set! {
    ParseError := {
        #[display("无法解析的URL: {0}")]
        Url(url::ParseError),
        #[display("无法解析的公共后缀: {0}")]
        PublicSuffix(publicsuffix::Error),
    }
    FrameworkError := {
        #[display("Tauri 框架错误: {0}")]
        Tauri(tauri::Error),
    }
    DatabaseError := {
        #[display("数据库执行错误: {0}")]
        Execute(sqlx::Error),
        #[display("数据库迁移错误: {0}")]
        Migrate(sqlx::migrate::MigrateError),
    }
    SetupError := {
        #[display("后台进程设置错误: {0}")]
        Task(delay_timer::error::TaskError),
    } || DatabaseError || FrameworkError || ParseError
    TabError := StateError || FrameworkError || ParseError
    StateError := {
        #[display("无法获取主视图")]
        NoMainView
    } || FrameworkError || DatabaseError || IconError
    IconError := {
        #[display("无法获取图标数据：{0}")]
        GetDataUrl(get_data_url::Error),
        #[display("无法保存图标：{0}")]
        SaveIcon(sqlx::Error),
        #[display("图标获取中")]
        Fetching,
    }
    SyncPublicSuffixError := {
        #[display("无法获取公共后缀: {0}")]
        FetchPublicSuffix(reqwest::Error)
    } || DatabaseError
    GetPublicSuffixError := ParseError || DatabaseError
}

impl_serialize![
    ParseError,
    DatabaseError,
    FrameworkError,
    TabError,
    StateError
];
