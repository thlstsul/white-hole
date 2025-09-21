use std::sync::Arc;
use std::{borrow::Borrow, fmt::Display, str::FromStr};

use error_set::error_set;
use keyboard_types::{Code, KeyState, Modifiers};
use log::error;
use scc::{HashMap, HashSet};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime, Wry, async_runtime};

use crate::browser::BrowserExt as _;

type HandlerFn<R> = Box<dyn Fn(&AppHandle<R>) + Send + Sync + 'static>;

pub fn setup() -> TauriPlugin<Wry> {
    Builder::new("hotkey-manager")
        .setup(move |app, _api| {
            let hotket_manager = HotkeyManager::new(app.clone());
            hotket_manager.register(Hotkey::new(Modifiers::ALT, Code::ArrowLeft), back);
            hotket_manager.register(Hotkey::new(Modifiers::ALT, Code::ArrowRight), forward);
            hotket_manager.register(Hotkey::new(Modifiers::CONTROL, Code::KeyT), focus);
            hotket_manager.register(Hotkey::new(Modifiers::CONTROL, Code::KeyL), focus);
            hotket_manager.register(Hotkey::new(Modifiers::empty(), Code::Escape), blur);
            hotket_manager.register(Hotkey::new(Modifiers::CONTROL, Code::KeyW), close_tab);
            hotket_manager.register(Hotkey::new(Modifiers::CONTROL, Code::Tab), next_tab);
            hotket_manager.register(Hotkey::new(Modifiers::empty(), Code::F11), fullscreen);
            app.manage(hotket_manager);
            Ok(())
        })
        .build()
}

fn back(app_handle: &AppHandle) {
    async_runtime::spawn({
        let app_handle = app_handle.clone();

        async move {
            let browser = app_handle.browser();
            browser.back().await;
        }
    });
}

fn forward(app_handle: &AppHandle) {
    async_runtime::spawn({
        let app_handle = app_handle.clone();

        async move {
            let browser = app_handle.browser();
            browser.forward().await;
        }
    });
}

fn focus(app_handle: &AppHandle) {
    async_runtime::spawn({
        let app_handle = app_handle.clone();

        async move {
            let browser = app_handle.browser();
            if let Err(e) = browser.focus().await {
                error!("浏览器焦点失败：{e}");
            }
        }
    });
}

fn blur(app_handle: &AppHandle) {
    async_runtime::spawn({
        let app_handle = app_handle.clone();

        async move {
            let browser = app_handle.browser();
            if let Err(e) = browser.blur().await {
                error!("浏览器焦点失败：{e}");
            }
        }
    });
}

fn close_tab(app_handle: &AppHandle) {
    async_runtime::spawn({
        let app_handle = app_handle.clone();

        async move {
            let browser = app_handle.browser();
            if let Err(e) = browser.close_tab().await {
                error!("关闭标签失败: {e}");
            }
        }
    });
}

fn next_tab(app_handle: &AppHandle) {
    async_runtime::spawn({
        let app_handle = app_handle.clone();

        async move {
            let browser = app_handle.browser();
            if let Err(e) = browser.next_tab().await {
                error!("浏览器切换标签失败：{e}");
            }
        }
    });
}

fn fullscreen(app_handle: &AppHandle) {
    async_runtime::spawn({
        let app_handle = app_handle.clone();

        async move {
            let browser = app_handle.browser();
            if let Err(e) = browser.fullscreen().await {
                error!("全屏失败: {e}");
            }
        }
    });
}

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

    pub fn register<F: Fn(&AppHandle<R>) + Send + Sync + 'static>(
        &self,
        hotkey: Hotkey,
        callback: F,
    ) {
        let _ = self
            .hotkeys
            .insert(hotkey, Arc::new(Box::new(callback)))
            .inspect_err(|(k, _)| error!("注册快捷键 {:?} 失败", k));
    }

    pub fn clear_pressed(&self) {
        self.pressed_keys.clear();
    }

    pub fn handle_key_event(&self, key: Code, state: KeyState) {
        if state == KeyState::Down {
            if self.pressed_keys.insert(key).is_ok() {
                // 键按下时检查快捷键
                self.check_hotkeys();
            }
        } else {
            self.pressed_keys.remove(&key);
        }
    }

    fn check_hotkeys(&self) {
        let Some(hotkey) = match_hotkey(&self.pressed_keys) else {
            return;
        };

        let Some(callback) = self.hotkeys.get(&hotkey) else {
            return;
        };
        callback(&self.app);
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

/// A keyboard shortcut that consists of an optional combination
/// of modifier keys (provided by [`Modifiers`](crate::hotkey::Modifiers)) and
/// one key ([`Code`](crate::hotkey::Code)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    /// The hotkey modifiers.
    pub mods: Modifiers,
    /// The hotkey key.
    pub key: Code,
}

impl Hotkey {
    /// Creates a new hotkey to define keyboard shortcuts throughout your application.
    /// Only [`Modifiers::ALT`], [`Modifiers::SHIFT`], [`Modifiers::CONTROL`], and [`Modifiers::META`]
    pub fn new(mut mods: Modifiers, key: Code) -> Self {
        if mods.contains(Modifiers::META) {
            mods.remove(Modifiers::META);
            mods.insert(Modifiers::META);
        }

        Self { mods, key }
    }

    /// Returns `true` if this [`Code`] and [`Modifiers`] matches this hotkey.
    #[allow(dead_code)]
    pub fn matches(&self, modifiers: impl Borrow<Modifiers>, key: impl Borrow<Code>) -> bool {
        // Should be a const but const bit_or doesn't work here.
        let base_mods = Modifiers::SHIFT | Modifiers::CONTROL | Modifiers::ALT | Modifiers::META;
        let modifiers = modifiers.borrow();
        let key = key.borrow();
        self.mods == *modifiers & base_mods && self.key == *key
    }

    /// Converts this hotkey into a string.
    pub fn into_string(self) -> String {
        let mut hotkey = String::new();
        if self.mods.contains(Modifiers::SHIFT) {
            hotkey.push_str("shift+")
        }
        if self.mods.contains(Modifiers::CONTROL) {
            hotkey.push_str("control+")
        }
        if self.mods.contains(Modifiers::ALT) {
            hotkey.push_str("alt+")
        }
        if self.mods.contains(Modifiers::META) {
            hotkey.push_str("super+")
        }
        hotkey.push_str(&self.key.to_string());
        hotkey
    }
}

impl Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.into_string())
    }
}

// Hotkey::from_str is available to be backward
// compatible with tauri and it also open the option
// to generate hotkey from string
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
    pressed_keys.scan(|code| match *code {
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
            let _ = key.insert(*code);
        }
    });

    key.map(|key| Hotkey { mods, key })
}

fn parse_hotkey(hotkey: &str) -> Result<Hotkey, HotkeyParseError> {
    let tokens = hotkey.split('+').collect::<Vec<&str>>();

    let mut mods = Modifiers::empty();
    let mut key = None;

    match tokens.len() {
        // single key hotkey
        1 => {
            key = Some(parse_key(tokens[0])?);
        }
        // modifiers and key comobo hotkey
        _ => {
            for raw in tokens {
                let token = raw.trim();

                if token.is_empty() {
                    return Err(HotkeyParseError::EmptyToken {
                        hotkey: hotkey.to_string(),
                    });
                }

                if key.is_some() {
                    // At this point we have parsed the modifiers and a main key, so by reaching
                    // this code, the function either received more than one main key or
                    //  the hotkey is not in the right order
                    // examples:
                    // 1. "Ctrl+Shift+C+A" => only one main key should be allowd.
                    // 2. "Ctrl+C+Shift" => wrong order
                    return Err(HotkeyParseError::InvalidFormat {
                        hotkey: hotkey.to_string(),
                    });
                }

                match token.to_uppercase().as_str() {
                    "OPTION" | "ALT" => {
                        mods |= Modifiers::ALT;
                    }
                    "CONTROL" | "CTRL" => {
                        mods |= Modifiers::CONTROL;
                    }
                    "COMMAND" | "CMD" | "META" => {
                        mods |= Modifiers::META;
                    }
                    "SHIFT" => {
                        mods |= Modifiers::SHIFT;
                    }
                    #[cfg(target_os = "macos")]
                    "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => {
                        mods |= Modifiers::META;
                    }
                    #[cfg(not(target_os = "macos"))]
                    "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => {
                        mods |= Modifiers::CONTROL;
                    }
                    _ => {
                        key = Some(parse_key(token)?);
                    }
                }
            }
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
    use Code::*;
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
    HotkeyParseError = {
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
    };
}
