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

    // `cargo test --lib` 的 harness 二进制不会获得 tauri-build 以 winres 资源方式
    // 嵌入的 RT_MANIFEST（那只进 bin 目标），因此加载器会绑定旧版 comctl32 v5；
    // 而 tauri 依赖树（muda 的 common-controls-v6 特性）静态导入了 v6 专属的
    // TaskDialogIndirect，v5 的导出表里没有该入口点，进程启动时直接报
    // STATUS_ENTRYPOINT_NOT_FOUND（0xc0000139）。
    //
    // MSVC 下改为把 Common-Controls v6 声明交给链接器
    // （/MANIFEST:EMBED + /MANIFESTDEPENDENCY），使所有链接产物（bin 与
    // 测试 harness）都带 v6 清单；同时不再让 tauri-build 以 winres 资源嵌入
    // 清单，避免 bin 出现重复的 RT_MANIFEST。清单内容与 tauri-build 默认的
    // windows-app-manifest.xml 完全一致。
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("pc-windows-msvc") {
        let attrs = tauri_build::Attributes::default()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        tauri_build::try_build(attrs).expect("failed to run tauri build script");

        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' \
             name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
             processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    } else {
        tauri_build::build()
    }
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
