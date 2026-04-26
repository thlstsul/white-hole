use sqlx::SqlitePool;

pub const DARKREADER_DISABLE_SCRIPT: &str = r#"DarkReader.auto(false)"#;
pub const DARKREADER_ENABLE_SCRIPT: &str = r#"
// 设置自定义 fetch 方法，使用 Tauri invoke 调用后端 fetch 以绕过 CORS 限制
DarkReader.setFetchMethod((url, options = {}) => {
    return window.__TAURI_INTERNALS__.invoke('fetch', { url, options }, { donotUseCustomProtocol: true })
        .then(resp => {
            return new Response(resp.body, {
                status: resp.status,
                statusText: resp.statusText,
                headers: resp.headers
            });
        });
});
DarkReader.auto({
  darkSchemeBackgroundColor: "\#1D232A",
  darkSchemeTextColor: "\#ECFAFF",
  lightSchemeBackgroundColor: "\#FFFFFF",
  lightSchemeTextColor: "\#18181B",
  brightness: 100,
  contrast: 90,
  sepia: 10,
})"#;

pub async fn switch(pool: &SqlitePool, host: &str) -> bool {
    sqlx::query!("select id from darkreader_blacklist where host = ?", host)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .is_none()
}

pub async fn save_blacklist(pool: &SqlitePool, host: &str) -> Result<i64, sqlx::Error> {
    sqlx::query!("insert into darkreader_blacklist (host) values (?)", host)
        .execute(pool)
        .await
        .map(|result| result.last_insert_rowid())
}

pub async fn delete_blacklist(pool: &SqlitePool, host: &str) -> Result<u64, sqlx::Error> {
    sqlx::query!("delete from darkreader_blacklist where host = ?", host)
        .execute(pool)
        .await
        .map(|result| result.rows_affected())
}
