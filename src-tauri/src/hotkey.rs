use keyboard_types::{Code, KeyState, Modifiers};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tauri::{AppHandle, Runtime};

type HandlerFn<R> = Box<dyn Fn(&AppHandle<R>, &Hotkey, KeyState) + Send + Sync + 'static>;

pub struct HotkeyManager<R: Runtime> {
    pressed_keys: HashSet<Code>,
    hotkeys: HashMap<Hotkey, Arc<HandlerFn<R>>>,
}

impl<R: Runtime> HotkeyManager<R> {
    pub fn new() -> Self {
        Self {
            pressed_keys: HashSet::new(),
            hotkeys: HashMap::new(),
        }
    }
}

fn codes2modifiers(pressed_codes: &HashSet<Code>) -> Modifiers {
    pressed_codes
        .into_iter()
        .filter_map(|c| {
            if *c == Code::ControlLeft || *c == Code::ControlRight {
                Some(Modifiers::CONTROL)
            } else if *c == Code::AltLeft || *c == Code::AltRight {
                Some(Modifiers::ALT)
            } else if *c == Code::ShiftLeft || *c == Code::ShiftRight {
                Some(Modifiers::SHIFT)
            } else if *c == Code::MetaLeft || *c == Code::MetaRight {
                Some(Modifiers::META)
            } else if *c == Code::Fn {
                Some(Modifiers::FN)
            } else if *c == Code::CapsLock {
                Some(Modifiers::CAPS_LOCK)
            } else if *c == Code::NumLock {
                Some(Modifiers::NUM_LOCK)
            } else if *c == Code::ScrollLock {
                Some(Modifiers::SCROLL_LOCK)
            } else if *c == Code::FnLock {
                Some(Modifiers::FN_LOCK)
            } else {
                None
            }
        })
        .fold(Modifiers::empty(), |acc, m| acc | m)
}
