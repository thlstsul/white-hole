//! 启动自检：确保应用已注册为 Windows 可识别的浏览器
//!
//! 安装器（NSIS/WiX）在安装时写入注册表；此处兜底修复绿色版运行、
//! 更换安装目录、注册表被清理等场景导致的注册失效。全部写入 HKCU，
//! 无需管理员权限。开发模式跳过，避免默认浏览器指向 debug 构建。

use std::io;

use log::{error, info};
use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, REG_SAM_FLAGS, REG_SZ, RRF_RT_REG_SZ, RegCloseKey,
    RegCreateKeyW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

const REG_KEY: PCWSTR = w!("SOFTWARE\\Clients\\StartMenuInternet\\white-hole.exe");
const REGISTERED_NAME: PCWSTR = w!("WhiteHole");
const REGISTERED_VALUE: &str = "SOFTWARE\\Clients\\StartMenuInternet\\white-hole.exe\\Capabilities";
const PROG_ID: &str = "WhiteHoleHTM";
const APP_NAME: &str = "White Hole";

pub fn setup() {
    if cfg!(debug_assertions) {
        return;
    }

    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(exe) = exe.to_str() else {
        return;
    };

    match is_registered(exe) {
        Ok(true) => {}
        Ok(false) => match register(exe) {
            Ok(()) => info!("浏览器注册已修复"),
            Err(e) => error!("修复浏览器注册失败：{e}"),
        },
        Err(e) => error!("检查浏览器注册失败：{e}"),
    }
}

/// 注册是否有效：StartMenuInternet 的 open command 指向当前 exe，
/// 且 RegisteredApplications 已声明。
fn is_registered(exe: &str) -> io::Result<bool> {
    let command = read_hkcu_default_str(w!(
        r"SOFTWARE\Clients\StartMenuInternet\white-hole.exe\shell\open\command"
    ))?;
    // 忽略大小写与正反斜杠差异：路径经快捷方式/系统展开时写法可能不同
    let normalize = |s: &str| s.to_lowercase().replace('/', "\\");
    if command.map(|c| normalize(&c)) != Some(normalize(&format!(r#""{exe}""#))) {
        return Ok(false);
    }

    let registered = read_hkcu_str(w!(r"SOFTWARE\RegisteredApplications"), REGISTERED_NAME)?;
    Ok(registered.as_deref() == Some(REGISTERED_VALUE))
}

/// 读取 HKCU 下某键的默认（未命名）字符串值；键或值不存在时返回 None
fn read_hkcu_default_str(subkey: PCWSTR) -> io::Result<Option<String>> {
    read_hkcu_str(subkey, PCWSTR::null())
}

/// 读取 HKCU 下某键的指定名称字符串值；键或值不存在时返回 None
fn read_hkcu_str(subkey: PCWSTR, value_name: PCWSTR) -> io::Result<Option<String>> {
    unsafe {
        let mut hkey = HKEY::default();
        let err = RegOpenKeyExW(HKEY_CURRENT_USER, subkey, None, KEY_QUERY_VALUE, &mut hkey);
        if err != ERROR_SUCCESS {
            return Ok(None);
        }

        let result = read_sz(hkey, value_name);
        let _ = RegCloseKey(hkey);
        result.map(Some)
    }
}

/// 读取键的 REG_SZ 值；value_name 为 NUL 时读默认（未命名）值
fn read_sz(hkey: HKEY, value_name: PCWSTR) -> io::Result<String> {
    unsafe {
        let mut len = 0u32;
        let err = RegGetValueW(
            hkey,
            PCWSTR::null(),
            value_name,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut len),
        );
        if err != ERROR_SUCCESS {
            return Err(io_error(err));
        }

        let mut buf = vec![0u16; len as usize / 2];
        let err = RegGetValueW(
            hkey,
            PCWSTR::null(),
            value_name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        );
        if err != ERROR_SUCCESS {
            return Err(io_error(err));
        }

        // len 含结尾 NUL；RegGetValueW 保证字符串已终止
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(String::from_utf16_lossy(&buf[..end]))
    }
}

fn register(exe: &str) -> io::Result<()> {
    // ---------- 1. StartMenuInternet ----------
    let root = create_key(HKEY_CURRENT_USER, REG_KEY)?;
    set_value(root.key(), None, APP_NAME)?;
    let cap = create_key(root.key(), w!("Capabilities"))?;
    set_value(cap.key(), w!("ApplicationName"), APP_NAME)?;
    set_value(
        cap.key(),
        w!("ApplicationDescription"),
        &format!("{APP_NAME} Browser"),
    )?;
    set_value(cap.key(), w!("ApplicationIcon"), &format!(r#""{exe}",0"#))?;
    let url_assoc = create_key(cap.key(), w!("URLAssociations"))?;
    set_value(url_assoc.key(), w!("http"), PROG_ID)?;
    set_value(url_assoc.key(), w!("https"), PROG_ID)?;
    let file_assoc = create_key(cap.key(), w!("FileAssociations"))?;
    set_value(file_assoc.key(), w!(".html"), PROG_ID)?;
    set_value(file_assoc.key(), w!(".htm"), PROG_ID)?;
    let icon = create_key(root.key(), w!("DefaultIcon"))?;
    set_value(icon.key(), None, &format!(r#""{exe}",0"#))?;
    let cmd = create_key(root.key(), w!("shell\\open\\command"))?;
    set_value(cmd.key(), None, &format!(r#""{exe}""#))?;

    // ---------- 2. ProgID ----------
    // 写 HKCU\Software\Classes（HKCR 的用户视图），无管理员权限也可写
    let prog_id = create_key(HKEY_CURRENT_USER, w!("Software\\Classes\\WhiteHoleHTM"))?;
    set_value(prog_id.key(), None, &format!("{APP_NAME} HTML Document"))?;
    let prog_id_icon = create_key(prog_id.key(), w!("DefaultIcon"))?;
    set_value(prog_id_icon.key(), None, &format!(r#""{exe}",0"#))?;
    let prog_id_cmd = create_key(prog_id.key(), w!("shell\\open\\command"))?;
    set_value(prog_id_cmd.key(), None, &format!(r#""{exe}" "%1""#))?;
    let prog_id_app = create_key(prog_id.key(), w!("Application"))?;
    set_value(
        prog_id_app.key(),
        w!("ApplicationIcon"),
        &format!(r#""{exe}",0"#),
    )?;

    // ---------- 3. RegisteredApplications ----------
    let reg_apps = create_key(HKEY_CURRENT_USER, w!("SOFTWARE\\RegisteredApplications"))?;
    set_value(reg_apps.key(), REGISTERED_NAME, REGISTERED_VALUE)?;

    Ok(())
}

/// HKEY 的 RAII 封装：离开作用域时自动 RegCloseKey
struct RegKeyHandle(HKEY);

impl RegKeyHandle {
    fn key(&self) -> HKEY {
        self.0
    }
}

impl Drop for RegKeyHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// 打开（不存在则创建）HKCU 下的子键；返回的句柄离开作用域自动关闭
fn create_key(parent: HKEY, subkey: PCWSTR) -> io::Result<RegKeyHandle> {
    let mut hkey = HKEY::default();
    let err = unsafe { RegCreateKeyW(parent, subkey, &mut hkey) };
    if err == ERROR_SUCCESS {
        Ok(RegKeyHandle(hkey))
    } else {
        Err(io_error(err))
    }
}

/// 写入 REG_SZ 值；name 为 None 时写默认（未命名）值
fn set_value<P: Into<Option<PCWSTR>>>(key: HKEY, name: P, value: &str) -> io::Result<()> {
    unsafe {
        let mut data = value.encode_utf16().collect::<Vec<_>>();
        data.push(0);
        let bytes: Vec<u8> = data.iter().flat_map(|c| c.to_le_bytes()).collect();

        // 未命名值对应名为 NUL 的宽字符串
        let name_buf: [u16; 1] = [0];
        let name = name.into().unwrap_or(PCWSTR(name_buf.as_ptr()));

        let err = RegSetValueExW(key, name, None, REG_SZ, Some(&bytes));
        if err == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io_error(err))
        }
    }
}

fn io_error(err: WIN32_ERROR) -> io::Error {
    io::Error::other(format!("Win32 错误码 {}", err.0))
}

const _: REG_SAM_FLAGS = KEY_QUERY_VALUE;
