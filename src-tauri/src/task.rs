use delay_timer::prelude::*;
use sqlx::SqlitePool;

use crate::{DB_URL, public_suffix::sync_public_suffix};

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
        let Ok(pool) = SqlitePool::connect(DB_URL.get().unwrap()).await else {
            return;
        };

        let _ = sync_public_suffix(&pool).await;
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
        let Ok(pool) = SqlitePool::connect(DB_URL.get().unwrap()).await else {
            return;
        };
        let _ = sync_public_suffix(&pool).await;
    };

    task_builder
        .set_task_id(2)
        .set_frequency_repeated_by_cron_str("0 0 10,15,21 * * *")
        .set_maximum_parallel_runnable_num(1)
        .spawn_async_routine(body)
}
