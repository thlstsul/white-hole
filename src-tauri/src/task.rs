use delay_timer::prelude::*;
use log::error;
use sqlx::SqlitePool;

use crate::{database::DB_PATH, public_suffix::sync_public_suffix};

pub fn setup() -> Result<(), TaskError> {
    let delay_timer = DelayTimerBuilder::default()
        .tokio_runtime_by_default()
        .build();

    delay_timer.add_task(startup_task()?)?;
    delay_timer.add_task(everyday_task()?)?;

    Ok(())
}

fn startup_task() -> Result<Task, TaskError> {
    let mut task_builder = TaskBuilder::default();
    let body = || async {
        let Some(db_path) = DB_PATH.get() else {
            return;
        };
        let Ok(pool) = SqlitePool::connect(&format!("sqlite:{db_path}")).await else {
            return;
        };

        if let Err(e) = sync_public_suffix(&pool).await {
            error!("同步 public suffix 失败：{e}");
        }
    };

    task_builder
        .set_task_id(1)
        .set_frequency_once_by_seconds(1)
        .set_maximum_parallel_runnable_num(1)
        .spawn_async_routine(body)
}

fn everyday_task() -> Result<Task, TaskError> {
    let mut task_builder = TaskBuilder::default();
    let body = || async move {
        let Some(db_path) = DB_PATH.get() else {
            return;
        };
        let Ok(pool) = SqlitePool::connect(&format!("sqlite:{db_path}")).await else {
            return;
        };

        if let Err(e) = sync_public_suffix(&pool).await {
            error!("同步 public suffix 失败：{e}");
        }

        if let Err(e) = crate::log::clear_log(&pool).await {
            error!("清理浏览记录失败：{e}");
        }

        if let Err(e) = crate::icon::clear_icon(&pool).await {
            error!("清理图标缓存失败：{e}");
        }
    };

    task_builder
        .set_task_id(2)
        .set_frequency_repeated_by_cron_str("0 0 10,15,21 * * *")
        .set_maximum_parallel_runnable_num(1)
        .spawn_async_routine(body)
}
