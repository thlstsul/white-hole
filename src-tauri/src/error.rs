use error_set::error_set;

use crate::impl_serialize;

error_set! {
    ParseError = {
        Url(url::ParseError),
    };
    FrameworkError = {
        Tauri(tauri::Error),
    };
    DatabaseError = {
        Execute(sqlx::Error),
        Migrate(sqlx::migrate::MigrateError),
    };
    SetupError = {
        DbConnect(sqlx::Error),
        Task(delay_timer::error::TaskError),
        Migarate(sqlx::migrate::MigrateError),
    } || FrameworkError || ParseError;
    TabError = StateError || FrameworkError || ParseError;
    StateError = {
        NoMainView
    } || FrameworkError || DatabaseError || IconError;
    WindowError = {
        WindowState(tauri_plugin_window_state::Error),
    } || FrameworkError || StateError;
    IconError = {
        GetDataUrl(get_data_url::Error),
        SaveIcon(sqlx::Error),
        Fetching,
    };
}

impl_serialize![
    ParseError,
    DatabaseError,
    FrameworkError,
    TabError,
    StateError
];
