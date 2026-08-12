//! 剪贴板历史（Win+V）兼容
//!
//! WebView2/Chromium 内部复制时，剪贴板所有者是 Chromium 的内部窗口，
//! Windows 剪贴板历史服务（Win+V）无法识别该所有者，导致复制的内容能
//! 正常粘贴但不出现在剪贴板历史里。解决办法：复制完成后，把剪贴板上的
//! 所有格式数据完整拷贝一遍，以宿主窗口为所有者重新设置，让历史服务能记录。

use log::warn;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};

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
