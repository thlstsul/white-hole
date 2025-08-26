use error_set::error_set;

use crate::impl_serialize;

error_set! {
    ParseError = {
        Url(url::ParseError),
    };
    LogError = {
        Execute(sqlx::Error),
    };
    FrameworkError = {
        Tauri(tauri::Error),
    };
    ShortcutError = {
        Shortcut(crate::shortcut::Error),
    };
    SetupError = {
        DbConnect(sqlx::Error),
        Task(delay_timer::error::TaskError),
        Migarate(sqlx::migrate::MigrateError),
    } || FrameworkError || ParseError || ShortcutError;
    TabError = StateError || FrameworkError || ParseError;
    StateError = {
        NoMainView
    } || FrameworkError || LogError || IconError;
    WindowError = {
        WindowState(tauri_plugin_window_state::Error),
    } || ShortcutError || FrameworkError || StateError;
    IconError = {
        GetDataUrl(get_data_url::Error),
        SaveIcon(sqlx::Error),
        Fetching,
    };
}

impl_serialize![ParseError, LogError, FrameworkError, TabError, StateError];
