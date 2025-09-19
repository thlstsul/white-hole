use keyboard::{Code, Modifiers};
use std::collections::HashSet;

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
            } else if *c == Code::Super {
                Some(Modifiers::SUPER)
            } else if *c == Code::Hyper {
                Some(Modifiers::HYPER)
            } else {
                None
            }
        })
        .fold(Modifiers::empty(), |acc, m| acc | m)
}
