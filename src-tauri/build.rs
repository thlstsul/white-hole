use std::io::Write as _;

fn main() {
    insert_public_suffix().expect("初始化 insert_public_suffix.sql 脚本失败");
    tauri_build::build()
}

fn insert_public_suffix() -> Result<(), Box<dyn std::error::Error>> {
    let public_suffix: String =
        reqwest::blocking::get("https://publicsuffix.org/list/public_suffix_list.dat")?.text()?;

    let sql = format!(
        "insert into public_suffix_list(create_time, content) values(datetime('now', 'localtime'), '{}');",
        public_suffix.replace("'", "''")
    );

    let mut sql_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open("../migrations/99999999999999_insert_public_suffix.sql")?;
    sql_file.write_all(sql.as_bytes())?;

    Ok(())
}
