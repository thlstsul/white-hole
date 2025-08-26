use std::time::Duration;

use cached::proc_macro::once;
use error_set::error_set;
use publicsuffix::List;
use sqlx::{SqlitePool, sqlite::SqliteQueryResult};

#[once(time = 90000, sync_writes = true, result = true)]
pub async fn get_public_suffix_cached(pool: &SqlitePool) -> Result<List, GetError> {
    let content = get_public_suffix(pool, false).await?;
    Ok(content.parse()?)
}

pub async fn sync_public_suffix(pool: &SqlitePool) -> Result<(), SyncError> {
    if get_public_suffix(pool, true).await.is_ok() {
        return Ok(());
    }

    let content: String = reqwest::get("https://publicsuffix.org/list/public_suffix_list.dat")
        .await?
        .text()
        .await?;

    update_public_suffix(pool, &content).await?;

    Ok(())
}

pub async fn get_public_suffix(pool: &SqlitePool, must_today: bool) -> Result<String, sqlx::Error> {
    if must_today {
        sqlx::query!(
            "select content from public_suffix_list where create_time > date('now', 'localtime') limit 1"
        )
        .fetch_one(pool)
        .await
        .map(|row| row.content)
    } else {
        sqlx::query!("select content from public_suffix_list limit 1")
            .fetch_one(pool)
            .await
            .map(|row| row.content)
    }
}

pub async fn update_public_suffix(
    pool: &SqlitePool,
    content: &str,
) -> Result<SqliteQueryResult, sqlx::Error> {
    sqlx::query!("delete from public_suffix_list;insert into public_suffix_list(content, create_time) values (?, datetime('now', 'localtime'))", content).execute(pool).await
}

error_set! {
    SyncError = {
        Get(reqwest::Error),
        Query(sqlx::Error),
    };
    GetError = {
        Parse(publicsuffix::Error),
        Query(sqlx::Error),
    };
}
