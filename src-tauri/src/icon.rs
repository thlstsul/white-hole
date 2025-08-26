use std::sync::OnceLock;

use cached::proc_macro::cached;
use get_data_url::GetDataUrl;
use sqlx::{SqlitePool, sqlite::SqliteQueryResult};
use tauri::async_runtime;

use crate::error::IconError;

static GET_DATA_URL: OnceLock<GetDataUrl> = OnceLock::new();

#[cached(key = "String", convert = r#"{ String::from(url) }"#, result = true)]
pub async fn get_icon_data_url(pool: &SqlitePool, url: &str) -> Result<String, IconError> {
    if let Ok(Some(record)) = sqlx::query!(
        "select data_url as 'data_url!' from icon_cached where url = ? and data_url like 'data:%' and update_time > datetime('now', '-1 month', 'localtime')",
        url
    )
    .fetch_optional(pool)
    .await
    {
        return Ok(record.data_url);
    }

    let pool = pool.clone();
    let url = url.to_owned();
    async_runtime::spawn(async move {
        let get_date_url = GET_DATA_URL.get_or_init(|| {
            let Ok(client) = reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36 Edg/139.0.0.0")
                .build() else {
                    return GetDataUrl::new();
                };
            GetDataUrl::with_client(client)
        });
        let Ok(data_url) = get_date_url
            .fetch(&url)
            .await
            .map(|data_url| data_url.to_string())
        else {
            return;
        };

        let _ = upsert_data_url(&pool, &url, &data_url).await;
    });

    Err(IconError::Fetching)
}

pub async fn save_icon(pool: &SqlitePool, url: String) -> Result<i64, sqlx::Error> {
    let id = get_id(pool, &url).await;
    if let Some(id) = id {
        return Ok(id);
    }

    sqlx::query!(
        "insert into icon_cached (url, data_url, update_time) values (?, ?, datetime('now', 'localtime'))",
        url,
        url
    ).execute(pool).await.map(|result| result.last_insert_rowid())
}

async fn upsert_data_url(
    pool: &SqlitePool,
    url: &str,
    data_url: &str,
) -> Result<SqliteQueryResult, sqlx::Error> {
    if let Some(id) = get_id(pool, url).await {
        sqlx::query!(
            "update icon_cached set data_url = ?, update_time = datetime('now', 'localtime')  where id = ?",
            data_url,
            id
        )
        .execute(pool)
        .await
    } else {
        sqlx::query!(
            "insert into icon_cached (url, data_url, update_time) values (?, ?, datetime('now', 'localtime'))",
            url,
            data_url
        ).execute(pool).await
    }
}

#[cached(key = "String", convert = r#"{ String::from(url) }"#, option = true)]
async fn get_id(pool: &SqlitePool, url: &str) -> Option<i64> {
    sqlx::query!(r#"select id as "id!" from icon_cached where url = ?"#, url)
        .fetch_optional(pool)
        .await
        .ok()?
        .map(|record| record.id)
}
