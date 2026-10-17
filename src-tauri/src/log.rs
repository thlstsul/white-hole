use cached::proc_macro::cached;
use log::error;
use serde::Serialize;
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use tauri::async_runtime;
use time::OffsetDateTime;

use crate::{
    icon::save_icon,
    page::{PageToken, Paginator as _},
    state::BrowserState,
    url::encode,
};

#[derive(Clone, Default, Serialize)]
pub struct QueryLogResponse {
    pub next_page_token: Option<PageToken>,
    pub logs: Vec<NavigationLog>,
}

#[derive(Clone, Default, Serialize, FromRow)]
pub struct NavigationLog {
    pub url: String,
    pub title: String,
    pub icon_url: String,
    pub star: bool,
    pub id: Option<i64>,
    pub last_time: Option<OffsetDateTime>,
}

/// incognito 参与缓存 key：无痕内存库与普通库的结果互不可见，
/// 避免无痕期间的数据经全局缓存泄露到普通模式（反之亦然）
pub async fn save_log(
    pool: &SqlitePool,
    incognito: bool,
    NavigationLog {
        url,
        title,
        icon_url,
        ..
    }: NavigationLog,
) -> Result<i64, sqlx::Error> {
    if url.is_empty() || url == "about:blank" {
        return Ok(-1);
    }

    let id = get_id(pool, incognito, &url).await;
    let icon_id = if !icon_url.is_empty() {
        save_icon(pool, incognito, &icon_url).await.unwrap_or(-1)
    } else {
        -1
    };

    let id = if let Some(id) = id {
        let mut builder: QueryBuilder<Sqlite> = QueryBuilder::new("");
        if !title.is_empty() {
            builder
                .push("update navigation_log set title = ")
                .push_bind(title);
            if icon_id != -1 {
                builder
                    .push(", icon_id = ")
                    .push_bind(icon_id)
                    .push(", times = times + 1, last_time = datetime('now', 'localtime') ");
            }
            builder.push("where id = ").push_bind(id);
        } else if icon_id != -1 {
            // 标题未就绪（页面加载早期）但图标已到位时，也要更新图标
            builder
                .push("update navigation_log set icon_id = ")
                .push_bind(icon_id)
                .push(", last_time = datetime('now', 'localtime') where id = ")
                .push_bind(id);
        } else {
            builder.push(
                "update navigation_log set last_time = datetime('now', 'localtime') where id = ",
            ).push_bind(id);
        }

        async_runtime::spawn({
            let pool = pool.clone();

            async move {
                if let Err(e) = builder.build().execute(&pool).await {
                    error!("更新浏览日志失败：{e}")
                }
            }
        });

        id
    } else {
        let result = sqlx::query!(
            "insert into navigation_log (url, title, icon_id, star, times, last_time) values (?, ?, ?, false, 0, datetime('now', 'localtime')) on conflict(url) do update set title = ?, icon_id = ?, times = times + 1",
            url,
            title,
            icon_id,
            title,
            icon_id,
        )
        .execute(pool)
        .await?;
        let mut id = result.last_insert_rowid();
        if id == 0 {
            id = get_id(pool, incognito, &url).await.unwrap_or(-1);
        }
        id
    };

    Ok(id)
}

/// 退出无痕模式时清除无痕侧缓存：无痕内存库已销毁，其结果（含无痕 URL）
/// 不应残留在进程内存的全局缓存中；普通侧条目仍然有效，予以保留
pub async fn clear_cache() {
    GET_URL.write().await.retain(|key, _| !key.0);
    GET_ID.write().await.retain(|key, _| !key.0);
}

#[cached(key = "(bool, i64)", convert = r#"{ (_incognito, id) }"#)]
pub async fn get_url(pool: &SqlitePool, _incognito: bool, id: i64) -> Option<String> {
    sqlx::query!("select url from navigation_log where id = ?", id)
        .fetch_optional(pool)
        .await
        .ok()?
        .map(|record| record.url)
}

#[cached(
    key = "(bool, String)",
    convert = r#"{ (_incognito, String::from(url)) }"#
)]
pub async fn get_id(pool: &SqlitePool, _incognito: bool, url: &str) -> Option<i64> {
    sqlx::query!(
        r#"select id as "id!" from navigation_log where url = ?"#,
        url
    )
    .fetch_optional(pool)
    .await
    .ok()?
    .map(|record| record.id)
}

pub async fn query_log(
    pool: &SqlitePool,
    keyword: &str,
    page_token: PageToken,
) -> Result<QueryLogResponse, sqlx::Error> {
    let mut query_builder: QueryBuilder<Sqlite> = QueryBuilder::new(
        "select a.id, a.url, a.title, b.data_url as icon_url, a.star, a.last_time from navigation_log a left outer join icon_cached b on a.icon_id = b.id where 1 = 1 ",
    );
    let mut is_empty = true;
    for keyword in keyword.split_whitespace() {
        if keyword.is_empty() {
            continue;
        }

        query_builder
            .push("and (a.url like ")
            .push_bind(format!("%{}%", encode(keyword).replace("%", "\\%")))
            .push(" escape '\\'")
            .push(" or a.title like ")
            .push_bind(format!("%{}%", keyword))
            .push(") ");
        is_empty = false;
    }

    if is_empty {
        query_builder.push("order by a.last_time desc ");
    } else {
        query_builder.push("order by a.star desc, a.times desc, length(a.url), a.last_time desc ");
    }

    query_builder.push(page_token.as_limit_sql());

    let mut logs = query_builder.build_query_as().fetch_all(pool).await?;

    Ok(QueryLogResponse {
        next_page_token: page_token.next_page(&mut logs),
        logs,
    })
}

pub async fn touch_log(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update navigation_log set last_time = datetime('now', 'localtime') where id = ?",
        id
    )
    .execute(pool)
    .await
    .map(|_| ())
}

pub async fn update_log_star(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!("update navigation_log set star = not star where id = ?", id)
        .execute(pool)
        .await
        .map(|_| ())
}

pub async fn clear_log(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query!("delete from navigation_log where url = title or title is null or title = ''")
        .execute(pool)
        .await
        .map(|_| ())
}

impl From<BrowserState> for NavigationLog {
    fn from(state: BrowserState) -> Self {
        Self {
            url: state.url,
            title: state.title,
            icon_url: state.icon_url,
            ..Default::default()
        }
    }
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
            let url = "https://cache-isolation.example.com/page";
            let log = |title: &str| NavigationLog {
                url: url.into(),
                title: title.into(),
                ..Default::default()
            };

            // 普通库写入后，普通侧命中并缓存；无痕侧查询空的内存库，返回 None
            let id = save_log(&normal, false, log("t1")).await.unwrap();
            assert!(id > 0);
            assert_eq!(get_id(&normal, false, url).await, Some(id));
            assert_eq!(get_url(&normal, false, id).await.as_deref(), Some(url));
            assert_eq!(get_id(&incognito, true, url).await, None);

            // 无痕内存库也写入同 URL，两侧条目各自独立存在于全局缓存
            save_log(&incognito, true, log("t2")).await.unwrap();
            assert!(get_id(&incognito, true, url).await.is_some());
            assert!(
                GET_ID
                    .write()
                    .await
                    .cache_get(&(true, url.to_string()))
                    .is_some(),
                "无痕侧条目应已入缓存"
            );
            assert!(
                GET_URL.write().await.cache_get(&(false, id)).is_some(),
                "普通侧条目应已入缓存"
            );

            // 退出无痕清理：无痕侧被清除，普通侧保留且仍命中
            clear_cache().await;
            assert!(
                GET_ID
                    .write()
                    .await
                    .cache_get(&(true, url.to_string()))
                    .is_none()
            );
            assert!(
                GET_ID
                    .write()
                    .await
                    .cache_get(&(false, url.to_string()))
                    .is_some(),
                "普通侧条目不应被清除"
            );
            assert_eq!(get_id(&normal, false, url).await, Some(id));
            assert_eq!(get_url(&normal, false, id).await.as_deref(), Some(url));
        });
    }
}
