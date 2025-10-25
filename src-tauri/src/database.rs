use std::sync::{Arc, OnceLock};

use sqlx::{
    SqlitePool,
    migrate::MigrateError,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tauri::{App, Manager as _, async_runtime::Mutex};

use crate::error::{DatabaseError, FrameworkError, SetupError};

pub const DB_NAME: &str = "white-hole.db";
pub static DB_PATH: OnceLock<String> = OnceLock::new();

pub fn get_db_path(app: &App) -> Result<&String, FrameworkError> {
    let db_path = app.path().app_local_data_dir()?.join(DB_NAME);
    Ok(DB_PATH.get_or_init(|| db_path.to_string_lossy().to_string()))
}

pub struct Database {
    storage: Arc<SqlitePool>,
    memory: Mutex<Option<Arc<SqlitePool>>>,
}

impl Database {
    pub async fn new(app: &App) -> Result<Self, SetupError> {
        let db_path = get_db_path(app)?;
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new().connect_with(options).await?;

        if let Err(MigrateError::VersionMismatch(version)) =
            sqlx::migrate!("../migrations").run(&pool).await
        {
            let migrator = sqlx::migrate!("../migrations");
            if let Some(checksum) = migrator.iter().find_map(|m| {
                if m.version == version {
                    Some(m.checksum.clone())
                } else {
                    None
                }
            }) {
                // 99999999999999_insert_public_suffix.sql 动态脚本
                let _ = sqlx::query("update _sqlx_migrations set checksum = ? where version = ?")
                    .bind(checksum.into_owned())
                    .bind(version)
                    .execute(&pool)
                    .await;
            }
            migrator.run(&pool).await?;
        }

        Ok(Self {
            storage: Arc::new(pool),
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
