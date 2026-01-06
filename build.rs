use std::fs::File;
use std::io;
use std::process::{Command, Stdio};
use std::thread;

fn main() {
    if is_release_build() {
        // dx、tauri 没有清空捆绑文件，会导致越编译越大
        let _ = std::fs::remove_dir_all("./target/dx");
        let _ = std::fs::remove_dir_all("./dist/public");
        println!("cargo:rerun-if-changed=./");
    }

    let _ = run_command_safely("dx", &["fmt"], None);

    let pnpm = if cfg!(windows) { "pnpm.cmd" } else { "pnpm" };

    // pnpm install tailwindcss daisyui
    run_command_safely(pnpm, &["install", "tailwindcss", "daisyui"], None)
        .expect("failed to execute pnpm");
}

fn run_command_safely(command: &str, args: &[&str], output_file: Option<&str>) -> io::Result<()> {
    // 根据编译模式选择执行方式
    if is_release_build() {
        // Release 模式：同步执行
        run_sync(command, args, output_file)?;
        // 同步执行完成后自动释放锁
    } else {
        // Debug 模式：后台执行
        spawn_detached(command, args, output_file)?;
    }

    Ok(())
}

fn is_release_build() -> bool {
    std::env::var("PROFILE")
        .map(|p| p == "release")
        .unwrap_or(false)
}

/// 同步执行命令
fn run_sync(command: &str, args: &[&str], output_file: Option<&str>) -> io::Result<()> {
    let mut cmd = Command::new(command);
    cmd.args(args);

    // 处理输出重定向
    match output_file {
        Some(path) => {
            let file = File::create(path)?;
            cmd.stdout(file.try_clone()?);
            cmd.stderr(file);
        }
        None => {
            cmd.stdout(Stdio::inherit());
            cmd.stderr(Stdio::inherit());
        }
    }

    let status = cmd.status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "Command failed with exit code: {:?}",
            status.code()
        )));
    }

    Ok(())
}

/// 后台执行命令
fn spawn_detached(command: &str, args: &[&str], output_file: Option<&str>) -> io::Result<()> {
    if cfg!(target_os = "windows") {
        spawn_windows(command, args, output_file)
    } else {
        spawn_unix(command, args, output_file)
    }
}

/// Windows 后台执行
fn spawn_windows(command: &str, args: &[&str], output_file: Option<&str>) -> io::Result<()> {
    let mut cmd_args = vec!["/C", "start", "/B", command];
    cmd_args.extend(args);

    let mut cmd = Command::new("cmd");
    cmd.args(&cmd_args);

    handle_output_redirect(&mut cmd, output_file)?;

    cmd.spawn()?;
    Ok(())
}

/// Unix 后台执行
fn spawn_unix(command: &str, args: &[&str], output_file: Option<&str>) -> io::Result<()> {
    // Convert to owned values to move into thread
    let command = command.to_owned();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let output_file = output_file.map(|s| s.to_string());

    thread::spawn(move || {
        let mut cmd = Command::new(&command);
        cmd.args(args.iter().map(String::as_str));

        // Convert Option<String> to Option<&str> when passing to function
        if let Err(e) = handle_output_redirect(&mut cmd, output_file.as_deref()) {
            eprintln!("Output redirect failed: {}", e);
            return;
        }

        match cmd.spawn() {
            Ok(mut child) => {
                let _ = child.wait();
            }
            Err(e) => {
                eprintln!("Failed to start process: {}", e);
            }
        }
    });

    Ok(())
}

/// 处理输出重定向
fn handle_output_redirect(cmd: &mut Command, output_file: Option<&str>) -> io::Result<()> {
    match output_file {
        Some(path) => {
            let file = File::create(path)?;
            cmd.stdout(file.try_clone()?);
            cmd.stderr(file);
        }
        None => {
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
        }
    }
    Ok(())
}
