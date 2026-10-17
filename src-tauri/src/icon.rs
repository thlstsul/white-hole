use std::sync::OnceLock;

use cached::proc_macro::cached;
use get_data_url::GetDataUrl;
use log::{error, info};
use reqwest::Client;
use sqlx::{SqlitePool, sqlite::SqliteQueryResult};
use tauri::async_runtime;

use crate::{error::IconError, user_agent::get_user_agent};

static GET_DATA_URL: OnceLock<GetDataUrl> = OnceLock::new();

/// incognito 参与缓存 key：无痕内存库与普通库的结果互不可见，
/// 避免无痕期间的数据经全局缓存泄露到普通模式（反之亦然）
#[cached(
    key = "(bool, String)",
    convert = r#"{ (_incognito, String::from(icon_url)) }"#
)]
pub async fn get_icon_data_url(
    pool: &SqlitePool,
    _incognito: bool,
    icon_url: &str,
) -> Result<String, IconError> {
    if let Ok(Some(record)) = sqlx::query!(
        "select data_url as 'data_url!' from icon_cached where url = ? and data_url like 'data:%' and update_time > datetime('now', '-1 month', 'localtime')",
        icon_url
    )
    .fetch_optional(pool)
    .await
    {
        return Ok(record.data_url);
    }

    async_runtime::spawn({
        let pool = pool.clone();
        let url = icon_url.to_owned();

        async move {
            let get_data_url = GET_DATA_URL.get_or_init(|| {
                let user_agent = get_user_agent();
                info!("User-Agent: {}", user_agent);
                let Ok(client) = Client::builder()
                    .user_agent(user_agent)
                    .referer(false)
                    .build()
                else {
                    return GetDataUrl::new();
                };
                GetDataUrl::with_client(client)
            });
            if let Ok(data_url) = get_data_url
                .fetch(&url)
                .await
                .map(|data_url| data_url.to_string())
                && !data_url.starts_with("data:text")
                && let Err(e) = upsert_data_url(&pool, &url, &data_url).await
            {
                error!("更新icon失败：{e}");
            }
        }
    });

    Err(IconError::Fetching)
}

#[cached(
    key = "(bool, String)",
    convert = r#"{ (_incognito, String::from(url)) }"#
)]
pub async fn get_cached_icon(pool: &SqlitePool, _incognito: bool, url: &str) -> Option<String> {
    sqlx::query!(
        "select a.data_url from icon_cached a, navigation_log b where a.id = b.icon_id and b.url = ?",
        url
    ).fetch_optional(pool).await.ok()?.map(|record| record.data_url)?
}

pub async fn save_icon(pool: &SqlitePool, incognito: bool, url: &str) -> Result<i64, sqlx::Error> {
    if let Some(id) = get_id(pool, incognito, url).await {
        return Ok(id);
    }

    sqlx::query!(
        "insert into icon_cached (url, data_url, update_time) values (?, ?, datetime('now', 'localtime'))",
        url,
        url
    ).execute(pool).await.map(|result| result.last_insert_rowid())
}

pub async fn clear_icon(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query!("delete from icon_cached where not exists (select 1 from navigation_log where icon_id = icon_cached.id)")
        .execute(pool)
        .await
        .map(|_| ())
}

async fn upsert_data_url(
    pool: &SqlitePool,
    url: &str,
    data_url: &str,
) -> Result<SqliteQueryResult, sqlx::Error> {
    sqlx::query!(
        "insert into icon_cached (url, data_url, update_time) values (?, ?, datetime('now', 'localtime')) on conflict(url) do update set data_url = excluded.data_url, update_time = excluded.update_time",
        url,
        data_url
    ).execute(pool).await
}

/// 退出无痕模式时清除无痕侧缓存：无痕内存库已销毁，其结果（含无痕 URL）
/// 不应残留在进程内存的全局缓存中；普通侧条目仍然有效，予以保留
pub async fn clear_cache() {
    GET_ICON_DATA_URL.write().await.retain(|key, _| !key.0);
    GET_CACHED_ICON.write().await.retain(|key, _| !key.0);
    GET_ID.write().await.retain(|key, _| !key.0);
}

#[cached(
    key = "(bool, String)",
    convert = r#"{ (_incognito, String::from(url)) }"#
)]
async fn get_id(pool: &SqlitePool, _incognito: bool, url: &str) -> Option<i64> {
    sqlx::query!(r#"select id as "id!" from icon_cached where url = ?"#, url)
        .fetch_optional(pool)
        .await
        .ok()?
        .map(|record| record.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cached::Cached as _;
    use sqlx::sqlite::SqlitePoolOptions;

    /// 单连接内存库：:memory: 每个连接是独立库，必须限 1 连接
    async fn memory_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../migrations").run(&pool).await.unwrap();
        pool
    }

    /// 隔离性 + 清空行为的回归测试：
    /// (true, …) 与 (false, …) 条目互不可见，clear_cache 只清除无痕侧。
    /// 合并为单个测试，避免并行测试间清空全局缓存互相干扰。
    #[test]
    fn incognito_cache_isolation_and_clear() {
        tauri::async_runtime::block_on(async {
            let normal = memory_pool().await;
            let incognito = memory_pool().await;
            let url = "https://icon-isolation.example.com/";
            let icon_url = "https://icon-isolation.example.com/favicon.ico";

            // 普通库：落历史 + 图标，普通侧命中并缓存
            let log = crate::log::save_log(
                &normal,
                false,
                crate::log::NavigationLog {
                    url: url.into(),
                    title: "t".into(),
                    icon_url: icon_url.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert!(log > 0);
            let icon_id = save_icon(&normal, false, icon_url).await.unwrap();
            assert!(icon_id > 0);
            assert_eq!(get_id(&normal, false, icon_url).await, Some(icon_id));
            assert_eq!(
                get_cached_icon(&normal, false, url).await.as_deref(),
                Some(icon_url)
            );

            // 无痕内存库写入同图标 URL，其条目独立存在于全局缓存
            let inc_icon_id = save_icon(&incognito, true, icon_url).await.unwrap();
            assert_eq!(get_id(&incognito, true, icon_url).await, Some(inc_icon_id));
            assert!(
                GET_ID
                    .write()
                    .await
                    .cache_get(&(true, icon_url.to_string()))
                    .is_some(),
                "无痕侧条目应已入缓存"
            );
            assert!(
                GET_CACHED_ICON
                    .write()
                    .await
                    .cache_get(&(false, url.to_string()))
                    .is_some(),
                "普通侧条目应已入缓存"
            );

            // 退出无痕清理：无痕侧被清除，普通侧保留且仍命中
            clear_cache().await;
            assert!(
                GET_ID
                    .write()
                    .await
                    .cache_get(&(true, icon_url.to_string()))
                    .is_none()
            );
            assert!(
                GET_CACHED_ICON
                    .write()
                    .await
                    .cache_get(&(false, url.to_string()))
                    .is_some(),
                "普通侧条目不应被清除"
            );
            assert_eq!(get_id(&normal, false, icon_url).await, Some(icon_id));
            assert_eq!(
                get_cached_icon(&normal, false, url).await.as_deref(),
                Some(icon_url)
            );
        });
    }
}
