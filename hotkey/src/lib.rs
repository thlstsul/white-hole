//! 快捷键管理：自实现的 Tauri 插件（替代 tauri-plugin-global-shortcut）。
//!
//! 管理器维护"当前按下键集合"，组合命中已注册快捷键时触发回调；
//! 注册经 [`inventory`](::inventory) 收集（[`hotkey!`] 宏生成
//! [`HotkeyRegistrar`]），插件 setup 时一次性挂载，无需逐个手动注册。
//! [`Hotkey`] 的字符串解析/显示兼容 tauri 格式（如 "Ctrl+Alt+K"）。

use std::sync::Arc;
use std::{borrow::Borrow, fmt::Display, str::FromStr};

use error_set::error_set;
pub use hotkey_macros::hotkey;
use inventory::iter;
pub use inventory::submit;
use keyboard_types::Code::*;
pub use keyboard_types::{Code, KeyState, Modifiers};
use log::error;
use scc::{HashMap, HashSet};
use std::fmt::{Formatter, Result as FmtResult};
use std::ops::Fn;
use tauri::plugin::{Builder as PluginBuilder, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime, Wry};

type HandlerFn<R> = Box<dyn Fn(AppHandle<R>) + Send + Sync + 'static>;

/// 构建插件：创建管理器，并把 inventory 收集到的全部注册器挂载上去
pub fn init() -> TauriPlugin<Wry> {
    PluginBuilder::new("hotkey-manager")
        .setup(move |app, _api| {
            let mut manager = HotkeyManager::new(app.clone());
            for factory in iter::<&dyn HotkeyRegistrar> {
                manager = factory.register(manager);
            }
            app.manage(manager);
            Ok(())
        })
        .build()
}

/// 注册器：`hotkey!` 宏为每个快捷键回调生成实现，经 inventory 收集后
/// 在插件 setup 时依次把注册项写进管理器（manager 按 `register` 返回值链式传递）
pub trait HotkeyRegistrar: Send + Sync + 'static {
    fn register(&self, manager: HotkeyManager<Wry>) -> HotkeyManager<Wry>;
}

inventory::collect!(&'static dyn HotkeyRegistrar);

#[macro_export]
macro_rules! submit_hotkey {
    ($ty:ident) => {
        ::hotkey::submit! {
            &$ty as &dyn ::hotkey::HotkeyRegistrar
        }
    };
}

/// 快捷键管理器：跟踪按下键集合并分发命中的快捷键回调
pub struct HotkeyManager<R: Runtime> {
    app: AppHandle<R>,
    pressed_keys: HashSet<Code>,
    hotkeys: HashMap<Hotkey, Arc<HandlerFn<R>>>,
}

impl<R: Runtime> HotkeyManager<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self {
            app,
            pressed_keys: HashSet::new(),
            hotkeys: HashMap::new(),
        }
    }

    pub fn init(app: AppHandle<R>, hotkeys: HashMap<Hotkey, Arc<HandlerFn<R>>>) -> Self {
        Self {
            app,
            pressed_keys: HashSet::new(),
            hotkeys,
        }
    }

    pub fn register<F: Fn(AppHandle<R>) + Send + Sync + 'static>(
        &self,
        hotkey: Hotkey,
        callback: F,
    ) {
        if let Err((k, _)) = self
            .hotkeys
            .insert_sync(hotkey, Arc::new(Box::new(callback)))
        {
            error!("注册快捷键 {k:?} 失败");
        }
    }

    pub fn clear_pressed(&self) {
        self.pressed_keys.clear_sync();
    }

    pub fn handle_key_event(&self, key: Code, state: KeyState) {
        if state == KeyState::Down {
            if self.pressed_keys.insert_sync(key).is_ok() {
                self.check_hotkeys();
            }
        } else {
            self.pressed_keys.remove_sync(&key);
        }
    }

    /// 查询某个键是否当前被按下（供外部判断修饰键状态，如 Ctrl+点击）
    pub fn is_pressed(&self, key: Code) -> bool {
        self.pressed_keys.contains_sync(&key)
    }

    fn check_hotkeys(&self) {
        let Some(hotkey) = match_hotkey(&self.pressed_keys) else {
            return;
        };

        let Some(callback) = self.hotkeys.get_sync(&hotkey) else {
            return;
        };
        callback(self.app.clone());
    }
}

pub trait HotkeyManagerExt<R: Runtime> {
    fn hotkey(&self) -> &HotkeyManager<R>;
}

impl<R: Runtime, T: Manager<R>> HotkeyManagerExt<R> for T {
    fn hotkey(&self) -> &HotkeyManager<R> {
        self.state::<HotkeyManager<R>>().inner()
    }
}

/// 一个快捷键组合：可选的修饰键（[`Modifiers`]）+ 一个主键（[`Code`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    /// 修饰键组合
    pub mods: Modifiers,
    /// 主键
    pub key: Code,
}

impl Hotkey {
    /// 创建快捷键组合
    pub fn new(mods: Modifiers, key: Code) -> Self {
        Self { mods, key }
    }

    /// 判断给定的修饰键与主键是否与自身匹配
    #[allow(dead_code)]
    pub fn matches(&self, modifiers: impl Borrow<Modifiers>, key: impl Borrow<Code>) -> bool {
        // 不能写成 const：const 上下文里位或运算不可用
        let base_mods = Modifiers::SHIFT | Modifiers::CONTROL | Modifiers::ALT | Modifiers::META;
        let modifiers = modifiers.borrow();
        let key = key.borrow();
        self.mods == *modifiers & base_mods && self.key == *key
    }

    /// 转为 "shift+control+a" 风格的字符串（修饰键按固定顺序在前）
    pub fn into_string(self) -> String {
        let mut hotkey = String::new();
        for (m, s) in [
            (Modifiers::SHIFT, "shift+"),
            (Modifiers::CONTROL, "control+"),
            (Modifiers::ALT, "alt+"),
            (Modifiers::META, "super+"),
        ] {
            if self.mods.contains(m) {
                hotkey.push_str(s);
            }
        }
        hotkey.push_str(&self.key.to_string());
        hotkey
    }
}

impl Display for Hotkey {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.into_string())
    }
}

// 提供 FromStr：与 tauri 的字符串快捷键格式向后兼容，也支持从字符串生成快捷键
impl FromStr for Hotkey {
    type Err = HotkeyParseError;
    fn from_str(hotkey_string: &str) -> Result<Self, Self::Err> {
        parse_hotkey(hotkey_string)
    }
}

impl TryFrom<&str> for Hotkey {
    type Error = HotkeyParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        parse_hotkey(value)
    }
}

impl TryFrom<String> for Hotkey {
    type Error = HotkeyParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        parse_hotkey(&value)
    }
}

fn match_hotkey(pressed_keys: &HashSet<Code>) -> Option<Hotkey> {
    let mut mods = Modifiers::empty();
    let mut key = None;
    pressed_keys.iter_sync(|code| {
        match *code {
            Code::ControlLeft | Code::ControlRight => mods |= Modifiers::CONTROL,
            Code::AltLeft | Code::AltRight => mods |= Modifiers::ALT,
            Code::ShiftLeft | Code::ShiftRight => mods |= Modifiers::SHIFT,
            Code::MetaLeft | Code::MetaRight => mods |= Modifiers::META,
            Code::Fn => mods |= Modifiers::FN,
            Code::CapsLock => mods |= Modifiers::CAPS_LOCK,
            Code::NumLock => mods |= Modifiers::NUM_LOCK,
            Code::ScrollLock => mods |= Modifiers::SCROLL_LOCK,
            Code::FnLock => mods |= Modifiers::FN_LOCK,
            _ => {
                key = Some(*code);
            }
        }
        true
    });

    key.map(|key| Hotkey { mods, key })
}

fn parse_hotkey(hotkey: &str) -> Result<Hotkey, HotkeyParseError> {
    let mut mods = Modifiers::empty();
    let mut key = None;

    for raw in hotkey.split('+') {
        let token = raw.trim();

        if token.is_empty() {
            return Err(HotkeyParseError::EmptyToken {
                hotkey: hotkey.to_string(),
            });
        }

        if key.is_some() {
            return Err(HotkeyParseError::InvalidFormat {
                hotkey: hotkey.to_string(),
            });
        }

        match token.to_uppercase().as_str() {
            "OPTION" | "ALT" => mods |= Modifiers::ALT,
            "CONTROL" | "CTRL" => mods |= Modifiers::CONTROL,
            "COMMAND" | "CMD" | "META" => mods |= Modifiers::META,
            "SHIFT" => mods |= Modifiers::SHIFT,
            #[cfg(target_os = "macos")]
            "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => {
                mods |= Modifiers::META;
            }
            #[cfg(not(target_os = "macos"))]
            "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => {
                mods |= Modifiers::CONTROL;
            }
            _ => key = Some(parse_key(token)?),
        }
    }

    Ok(Hotkey::new(
        mods,
        key.ok_or_else(|| HotkeyParseError::InvalidFormat {
            hotkey: hotkey.to_string(),
        })?,
    ))
}

fn parse_key(key: &str) -> Result<Code, HotkeyParseError> {
    match key.to_uppercase().as_str() {
        "BACKQUOTE" | "`" => Ok(Backquote),
        "BACKSLASH" | "\\" => Ok(Backslash),
        "BRACKETLEFT" | "[" => Ok(BracketLeft),
        "BRACKETRIGHT" | "]" => Ok(BracketRight),
        "PAUSE" | "PAUSEBREAK" => Ok(Pause),
        "COMMA" | "," => Ok(Comma),
        "DIGIT0" | "0" => Ok(Digit0),
        "DIGIT1" | "1" => Ok(Digit1),
        "DIGIT2" | "2" => Ok(Digit2),
        "DIGIT3" | "3" => Ok(Digit3),
        "DIGIT4" | "4" => Ok(Digit4),
        "DIGIT5" | "5" => Ok(Digit5),
        "DIGIT6" | "6" => Ok(Digit6),
        "DIGIT7" | "7" => Ok(Digit7),
        "DIGIT8" | "8" => Ok(Digit8),
        "DIGIT9" | "9" => Ok(Digit9),
        "EQUAL" | "=" => Ok(Equal),
        "KEYA" | "A" => Ok(KeyA),
        "KEYB" | "B" => Ok(KeyB),
        "KEYC" | "C" => Ok(KeyC),
        "KEYD" | "D" => Ok(KeyD),
        "KEYE" | "E" => Ok(KeyE),
        "KEYF" | "F" => Ok(KeyF),
        "KEYG" | "G" => Ok(KeyG),
        "KEYH" | "H" => Ok(KeyH),
        "KEYI" | "I" => Ok(KeyI),
        "KEYJ" | "J" => Ok(KeyJ),
        "KEYK" | "K" => Ok(KeyK),
        "KEYL" | "L" => Ok(KeyL),
        "KEYM" | "M" => Ok(KeyM),
        "KEYN" | "N" => Ok(KeyN),
        "KEYO" | "O" => Ok(KeyO),
        "KEYP" | "P" => Ok(KeyP),
        "KEYQ" | "Q" => Ok(KeyQ),
        "KEYR" | "R" => Ok(KeyR),
        "KEYS" | "S" => Ok(KeyS),
        "KEYT" | "T" => Ok(KeyT),
        "KEYU" | "U" => Ok(KeyU),
        "KEYV" | "V" => Ok(KeyV),
        "KEYW" | "W" => Ok(KeyW),
        "KEYX" | "X" => Ok(KeyX),
        "KEYY" | "Y" => Ok(KeyY),
        "KEYZ" | "Z" => Ok(KeyZ),
        "MINUS" | "-" => Ok(Minus),
        "PERIOD" | "." => Ok(Period),
        "QUOTE" | "'" => Ok(Quote),
        "SEMICOLON" | ";" => Ok(Semicolon),
        "SLASH" | "/" => Ok(Slash),
        "BACKSPACE" => Ok(Backspace),
        "CAPSLOCK" => Ok(CapsLock),
        "ENTER" => Ok(Enter),
        "SPACE" => Ok(Space),
        "TAB" => Ok(Tab),
        "DELETE" => Ok(Delete),
        "END" => Ok(End),
        "HOME" => Ok(Home),
        "INSERT" => Ok(Insert),
        "PAGEDOWN" => Ok(PageDown),
        "PAGEUP" => Ok(PageUp),
        "PRINTSCREEN" => Ok(PrintScreen),
        "SCROLLLOCK" => Ok(ScrollLock),
        "ARROWDOWN" | "DOWN" => Ok(ArrowDown),
        "ARROWLEFT" | "LEFT" => Ok(ArrowLeft),
        "ARROWRIGHT" | "RIGHT" => Ok(ArrowRight),
        "ARROWUP" | "UP" => Ok(ArrowUp),
        "NUMLOCK" => Ok(NumLock),
        "NUMPAD0" | "NUM0" => Ok(Numpad0),
        "NUMPAD1" | "NUM1" => Ok(Numpad1),
        "NUMPAD2" | "NUM2" => Ok(Numpad2),
        "NUMPAD3" | "NUM3" => Ok(Numpad3),
        "NUMPAD4" | "NUM4" => Ok(Numpad4),
        "NUMPAD5" | "NUM5" => Ok(Numpad5),
        "NUMPAD6" | "NUM6" => Ok(Numpad6),
        "NUMPAD7" | "NUM7" => Ok(Numpad7),
        "NUMPAD8" | "NUM8" => Ok(Numpad8),
        "NUMPAD9" | "NUM9" => Ok(Numpad9),
        "NUMPADADD" | "NUMADD" | "NUMPADPLUS" | "NUMPLUS" => Ok(NumpadAdd),
        "NUMPADDECIMAL" | "NUMDECIMAL" => Ok(NumpadDecimal),
        "NUMPADDIVIDE" | "NUMDIVIDE" => Ok(NumpadDivide),
        "NUMPADENTER" | "NUMENTER" => Ok(NumpadEnter),
        "NUMPADEQUAL" | "NUMEQUAL" => Ok(NumpadEqual),
        "NUMPADMULTIPLY" | "NUMMULTIPLY" => Ok(NumpadMultiply),
        "NUMPADSUBTRACT" | "NUMSUBTRACT" => Ok(NumpadSubtract),
        "ESCAPE" | "ESC" => Ok(Escape),
        "F1" => Ok(F1),
        "F2" => Ok(F2),
        "F3" => Ok(F3),
        "F4" => Ok(F4),
        "F5" => Ok(F5),
        "F6" => Ok(F6),
        "F7" => Ok(F7),
        "F8" => Ok(F8),
        "F9" => Ok(F9),
        "F10" => Ok(F10),
        "F11" => Ok(F11),
        "F12" => Ok(F12),
        "AUDIOVOLUMEDOWN" | "VOLUMEDOWN" => Ok(AudioVolumeDown),
        "AUDIOVOLUMEUP" | "VOLUMEUP" => Ok(AudioVolumeUp),
        "AUDIOVOLUMEMUTE" | "VOLUMEMUTE" => Ok(AudioVolumeMute),
        "MEDIAPLAY" => Ok(MediaPlay),
        "MEDIAPAUSE" => Ok(MediaPause),
        "MEDIAPLAYPAUSE" => Ok(MediaPlayPause),
        "MEDIASTOP" => Ok(MediaStop),
        "MEDIATRACKNEXT" => Ok(MediaTrackNext),
        "MEDIATRACKPREV" | "MEDIATRACKPREVIOUS" => Ok(MediaTrackPrevious),
        "F13" => Ok(F13),
        "F14" => Ok(F14),
        "F15" => Ok(F15),
        "F16" => Ok(F16),
        "F17" => Ok(F17),
        "F18" => Ok(F18),
        "F19" => Ok(F19),
        "F20" => Ok(F20),
        "F21" => Ok(F21),
        "F22" => Ok(F22),
        "F23" => Ok(F23),
        "F24" => Ok(F24),

        _ => Err(HotkeyParseError::UnsupportedKey {
            key: key.to_string(),
        }),
    }
}

error_set! {
    HotkeyParseError := {
        #[display("Couldn't recognize \"{key}\" as a valid key for hotkey, if you feel like it should be, please report this to https://github.com/tauri-apps/muda")]
        UnsupportedKey {
            key: String,
        },
        #[display("Found empty token while parsing hotkey: {hotkey}")]
        EmptyToken {
            hotkey: String,
        },
        #[display("Invalid hotkey format: \"{hotkey}\", an hotkey should have the modifiers first and only one main key, for example: \"Shift + Alt + K\"")]
        InvalidFormat {
            hotkey: String,
        },
    }
}
