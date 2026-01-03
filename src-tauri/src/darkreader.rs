use sqlx::{SqlitePool, sqlite::SqliteQueryResult};

pub const DARKREADER_DISABLE_SCRIPT: &str = r#"DarkReader.auto(false)"#;
pub const DARKREADER_ENABLE_SCRIPT: &str = r#"DarkReader.auto({
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
