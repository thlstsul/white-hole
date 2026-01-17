use std::{io::Write as _, thread};

fn main() {
    if is_release_build() {
        let darkreader_handler =
            thread::spawn(|| darkreader().expect("下载 darkreader.js 脚本失败"));
        let public_suffix = thread::spawn(|| {
            insert_public_suffix().expect("初始化 insert_public_suffix.sql 脚本失败")
        });
        let (_, _) = (darkreader_handler.join(), public_suffix.join());
        println!("cargo:rerun-if-changed=./");
    }

    tauri_build::build()
}

fn darkreader() -> Result<(), Box<dyn std::error::Error>> {
    let darkreader: String =
        reqwest::blocking::get("https://unpkg.com/darkreader@latest/darkreader.js")?.text()?;

    let mut darkreader_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open("js/darkreader.js")?;
    darkreader_file.write_all(darkreader.as_bytes())?;

    Ok(())
}

fn insert_public_suffix() -> Result<(), Box<dyn std::error::Error>> {
    let public_suffix: String =
        reqwest::blocking::get("https://publicsuffix.org/list/public_suffix_list.dat")?.text()?;

    let sql = format!(
        "insert into public_suffix_list (create_time, content) values (datetime('now', 'localtime'), '{}');",
        public_suffix.replace("'", "''")
    );

    let mut sql_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open("../migrations/99999999999999_insert_public_suffix.sql")?;
    sql_file.write_all(sql.as_bytes())?;

    Ok(())
}

fn is_release_build() -> bool {
    std::env::var("PROFILE")
        .map(|p| p == "release")
        .unwrap_or(false)
}
