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

pub async fn save_log(
    pool: &SqlitePool,
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

    let id = get_id(pool, &url).await;
    let icon_id = if !icon_url.is_empty() {
        save_icon(pool, &icon_url).await.unwrap_or(-1)
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
        } else {
            builder.push(
                "update navigation_log set last_time = datetime('now', 'localtime') where id = ",
            ).push_bind(id);
        }

        async_runtime::spawn({
            let pool = pool.clone();

            async move {
                if let Err(e) = builder.build().execute(&pool).await {
                    error!("更新浏览日志失败: {e}")
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
            id = get_id(pool, &url).await.unwrap_or(-1);
        }
        id
    };

    Ok(id)
}

#[cached(key = "i64", convert = r#"{ id }"#, option = true)]
pub async fn get_url(pool: &SqlitePool, id: i64) -> Option<String> {
    sqlx::query!("select url from navigation_log where id = ?", id)
        .fetch_optional(pool)
        .await
        .ok()?
        .map(|record| record.url)
}

#[cached(key = "String", convert = r#"{ String::from(url) }"#, option = true)]
pub async fn get_id(pool: &SqlitePool, url: &str) -> Option<i64> {
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
    let mut query_builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
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

pub async fn update_log_star(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!("update navigation_log set star = not star where id = ?", id)
        .execute(pool)
        .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn query_log_by_id(
    pool: &SqlitePool,
    ids: &[i64],
) -> Result<Vec<NavigationLog>, sqlx::Error> {
    let mut query_builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "select a.id, a.url, a.title, b.data_url as icon_url, a.star, a.last_time from navigation_log a left outer join icon_cached b on a.icon_id = b.id where a.id in (",
    );
    let mut separated = query_builder.separated(", ");
    for id in ids {
        separated.push_bind(*id);
    }
    separated.push_unseparated(") ");

    let record = query_builder.build_query_as().fetch_all(pool).await?;

    Ok(record)
}

pub async fn clear_log(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // 清理跳转的URL记录
    sqlx::query!("delete from navigation_log where url = title or title is null or title = ''")
        .execute(pool)
        .await?;
    Ok(())
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
