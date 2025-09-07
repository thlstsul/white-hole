use std::sync::{Arc, OnceLock};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tauri::{App, Manager as _, async_runtime::Mutex};

use crate::error::{DatabaseError, FrameworkError, SetupError};

pub const DB_NAME: &str = "white-hole.db";
pub static DB_URL: OnceLock<String> = OnceLock::new();

pub fn get_db_url(app: &App) -> Result<&String, FrameworkError> {
    let data_path = app.path().app_local_data_dir()?;
    let db_path = data_path.join(DB_NAME);
    Ok(DB_URL.get_or_init(|| format!("sqlite:{}", db_path.to_string_lossy())))
}

pub struct Database {
    storage: Arc<SqlitePool>,
    memory: Mutex<Option<Arc<SqlitePool>>>,
}

impl Database {
    pub async fn new(app: &App) -> Result<Self, SetupError> {
        let db_path = app.path().app_local_data_dir()?.join(DB_NAME);
        if !db_path.exists() {
            let options = SqliteConnectOptions::new()
                .filename(db_path)
                .create_if_missing(true)
                .foreign_keys(true);

            let pool = SqlitePoolOptions::new().connect_with(options).await?;
            sqlx::migrate!("../migrations").run(&pool).await?;
        }

        Ok(Self {
            storage: Arc::new(SqlitePool::connect(get_db_url(app)?).await?),
            memory: Mutex::new(None),
        })
    }

    pub async fn get(&self) -> Arc<SqlitePool> {
        let guard = self.memory.lock().await;
        guard.as_ref().unwrap_or(&self.storage).clone()
    }

    pub async fn migrate_memory(&self) -> Result<(), DatabaseError> {
        let pool = SqlitePool::connect("sqlite::memory:").await?;
        sqlx::migrate!("../migrations").run(&pool).await?;
        let mut memory = self.memory.lock().await;
        *memory = Some(Arc::new(pool));
        Ok(())
    }

    pub async fn close_memory(&self) -> Result<(), DatabaseError> {
        let Some(pool) = self.memory.lock().await.take() else {
            return Ok(());
        };

        pool.close().await;
        Ok(())
    }
}
