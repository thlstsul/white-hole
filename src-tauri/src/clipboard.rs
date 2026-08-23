//! 剪贴板历史（Win+V）兼容
//!
//! WebView2/Chromium 内部复制时，剪贴板所有者是 Chromium 的内部窗口，
//! Windows 剪贴板历史服务（Win+V）无法识别该所有者，导致复制的内容能
//! 正常粘贴但不出现在剪贴板历史里。解决办法：复制完成后，把剪贴板上的
//! 所有格式数据完整拷贝一遍，以宿主窗口为所有者重新设置，让历史服务能记录。
//!
//! 触发方式：不再由页面 JS 调用 IPC 命令（remote 权限集已移除该命令），
//! 而是由 [`watch`] 启动的后台线程轮询系统剪贴板序列号，检测到本应用
//! WebView2 进程发起的复制后，自行执行重接管（带节流）。

use std::time::Duration;

use log::warn;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{CloseHandle, GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardOwner,
    GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

/// 剪贴板轮询间隔：序列号变化即触发，间隔只需覆盖手动复制频率
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// 剪贴板重接管的最小间隔：防止高频复制时过度占用剪贴板
const REOWN_COOLDOWN: Duration = Duration::from_millis(300);

/// 启动剪贴板监听线程：轮询系统剪贴板序列号，检测到本应用 WebView2
/// 进程发起的复制后，由宿主自行重新接管剪贴板。
/// 只处理本应用发起的复制（所有者进程过滤），避免干扰其他程序。
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_seq = unsafe { GetClipboardSequenceNumber() };
        let mut last_reown = std::time::Instant::now()
            .checked_sub(REOWN_COOLDOWN)
            .unwrap_or_else(std::time::Instant::now);

        loop {
            std::thread::sleep(POLL_INTERVAL);
            let seq = unsafe { GetClipboardSequenceNumber() };
            if seq == last_seq {
                continue;
            }
            last_seq = seq;

            // 只处理本应用 WebView2 发起的复制
            if !is_our_webview_owner() {
                continue;
            }

            // 冷却期内跳过，避免高频复制时过度占用剪贴板
            if last_reown.elapsed() < REOWN_COOLDOWN {
                continue;
            }

            let Some(hwnd) = app.get_window("main").and_then(|w| w.hwnd().ok()) else {
                continue;
            };
            last_reown = std::time::Instant::now();
            reown(Some(hwnd));
        }
    });
}

/// 当前剪贴板所有者是否为本应用 WebView2 子进程
fn is_our_webview_owner() -> bool {
    let Ok(owner) = (unsafe { GetClipboardOwner() }) else {
        return false;
    };
    if owner.is_invalid() {
        return false;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(owner, Some(&mut pid)) };
    if pid == 0 {
        return false;
    }
    our_webview_pids().contains(&pid)
}

/// 枚举本应用启动的 WebView2 子进程 PID（进程名 msedgewebview2.exe）
fn our_webview_pids() -> Vec<u32> {
    let mut pids = Vec::new();
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return pids;
    };

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry).is_ok() };
    while ok {
        // 父进程是本进程 且 进程名为 WebView2 浏览器进程
        if entry.th32ParentProcessID == std::process::id() && is_webview2_exe(&entry.szExeFile) {
            pids.push(entry.th32ProcessID);
        }
        ok = unsafe { Process32NextW(snapshot, &mut entry).is_ok() };
    }
    let _ = unsafe { CloseHandle(snapshot) };
    pids
}

/// 判断进程可执行文件名是否为 WebView2 浏览器进程
fn is_webview2_exe(exe: &[u16; 260]) -> bool {
    let len = exe.iter().position(|&c| c == 0).unwrap_or(exe.len());
    let name = String::from_utf16_lossy(&exe[..len]).to_lowercase();
    name == "msedgewebview2.exe"
}

/// 该格式的数据不是 HGLOBAL（GlobalLock 会失效），跳过：
/// 2=CF_BITMAP(HBITMAP) 3=CF_METAFILEPICT 9=CF_PALETTE(HPALETTE)
/// 14=CF_ENHMETAFILE 0x80=CF_OWNERDISPLAY，0x81-0xFF=显示器格式（CF_DSP*）
fn is_global_memory_format(format: u32) -> bool {
    !matches!(format, 2 | 3 | 9 | 14 | 0x0080) && !(0x0081..=0x00FF).contains(&format)
}

/// 以宿主窗口为所有者重新设置剪贴板，使内容出现在 Win+V 剪贴板历史中
pub fn reown(hwnd: Option<HWND>) {
    // 缺少有效窗口句柄时放弃：OpenClipboard(NULL) 下 EmptyClipboard 会把
    // 所有者设为 NULL，导致 SetClipboardData 全部失败并清空剪贴板
    let Some(hwnd) = hwnd else {
        return;
    };

    unsafe {
        // 其他程序正在占用剪贴板时放弃，等下一次复制再处理
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }

        // 先枚举并读取当前所有格式的数据（必须在清空前完成读取）
        let mut entries: Vec<(u32, Vec<u8>)> = Vec::new();
        // 存在无法完整复制的格式时中止重设，避免 EmptyClipboard 销毁它们
        let mut skipped = false;
        let mut format = 0;
        loop {
            format = EnumClipboardFormats(format);
            if format == 0 {
                break;
            }
            if !is_global_memory_format(format) {
                skipped = true;
                continue;
            }
            let Ok(data) = GetClipboardData(format) else {
                skipped = true;
                continue;
            };
            let handle = HGLOBAL(data.0);
            let size = GlobalSize(handle);
            let ptr = GlobalLock(handle);
            if !ptr.is_null() {
                let bytes = std::slice::from_raw_parts(ptr as *const u8, size);
                entries.push((format, bytes.to_vec()));
            } else {
                skipped = true;
            }
            let _ = GlobalUnlock(handle);
        }

        // 没有可重设的数据，或存在无法复制的格式时，不动剪贴板（避免清空已有内容）
        if entries.is_empty() || skipped {
            let _ = CloseClipboard();
            return;
        }

        if EmptyClipboard().is_err() {
            let _ = CloseClipboard();
            return;
        }

        for (format, bytes) in &entries {
            let Ok(handle) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else {
                warn!("分配剪贴板内存失败：format={format}");
                continue;
            };
            let ptr = GlobalLock(handle);
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
                let _ = GlobalUnlock(handle);
                if SetClipboardData(*format, Some(HANDLE(handle.0))).is_err() {
                    let _ = GlobalFree(Some(handle));
                }
                // 成功后句柄由系统接管，不再释放
            } else {
                let _ = GlobalFree(Some(handle));
            }
        }

        let _ = CloseClipboard();
    }
}
