use delay_timer::prelude::*;
use log::error;
use sqlx::SqlitePool;

use crate::{database::DB_PATH, public_suffix::sync_public_suffix};

pub fn setup() -> Result<(), TaskError> {
    let delay_timer = DelayTimerBuilder::default()
        .tokio_runtime_by_default()
        .build();

    delay_timer.add_task(sync_suffix_startup()?)?;
    delay_timer.add_task(sync_suffix_everyday()?)?;
    delay_timer.add_task(clear_log_everyday()?)?;
    delay_timer.add_task(clear_icon_everyday()?)?;

    Ok(())
}

async fn connect_db() -> Option<SqlitePool> {
    let db_path = DB_PATH.get()?;
    SqlitePool::connect(&format!("sqlite:{db_path}")).await.ok()
}

async fn sync_suffix() {
    let Some(pool) = connect_db().await else {
        return;
    };
    if let Err(e) = sync_public_suffix(&pool).await {
        error!("同步 public suffix 失败：{e}");
    }
}

fn sync_suffix_startup() -> Result<Task, TaskError> {
    TaskBuilder::default()
        .set_task_id(1)
        .set_frequency_once_by_seconds(1)
        .set_maximum_parallel_runnable_num(1)
        .spawn_async_routine(sync_suffix)
}

fn sync_suffix_everyday() -> Result<Task, TaskError> {
    TaskBuilder::default()
        .set_task_id(2)
        .set_frequency_repeated_by_cron_str("0 0 10,15,21 * * *")
        .set_maximum_parallel_runnable_num(1)
        .spawn_async_routine(sync_suffix)
}

fn clear_log_everyday() -> Result<Task, TaskError> {
    TaskBuilder::default()
        .set_task_id(3)
        .set_frequency_repeated_by_cron_str("0 0 10,15,21 * * *")
        .set_maximum_parallel_runnable_num(1)
        .spawn_async_routine(|| async {
            let Some(pool) = connect_db().await else {
                return;
            };

            if let Err(e) = crate::log::clear_log(&pool).await {
                error!("清理浏览记录失败：{e}");
            }
        })
}

fn clear_icon_everyday() -> Result<Task, TaskError> {
    TaskBuilder::default()
        .set_task_id(4)
        .set_frequency_repeated_by_cron_str("0 0 10,15,21 * * *")
        .set_maximum_parallel_runnable_num(1)
        .spawn_async_routine(|| async {
            let Some(pool) = connect_db().await else {
                return;
            };

            if let Err(e) = crate::icon::clear_icon(&pool).await {
                error!("清理图标缓存失败：{e}");
            }
        })
}
