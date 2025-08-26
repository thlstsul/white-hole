use crate::{
    app::Browser, navigation::Navigator, url::percent_decode_str,
    window_decoration::WindowDecoration,
};
use dioxus::{html::input_data::MouseButton, prelude::*};
use tauri_sys::core::invoke;

const DEFAULT_ICON: Asset = asset!("/assets/default_icon.svg");

#[component]
pub fn TitleBar() -> Element {
    let start_dragging = move |e: MouseEvent| async move {
        let Some(button) = e.trigger_button() else {
            return;
        };

        if button == MouseButton::Primary {
            invoke::<()>("start_dragging", &()).await;
        }
    };

    rsx! {
        div {
            class: "title-bar navbar min-h-10 h-10",
            onmousedown: start_dragging,

            Navigator {}
            TitleBarContent {}
            WindowDecoration { class: "fixed top-0 right-0" }
        }
    }
}

#[component]
fn TitleBarContent(#[props(default)] class: String) -> Element {
    let browser = use_context::<Browser>();

    let focus = |_| async move {
        invoke::<()>("focus", &()).await;
    };

    rsx! {
        div {
            class: "title-bar-content flex items-center max-w-2/3 {class}",
            onclick: focus,
            onmousedown: |e| e.stop_propagation(),

            Icon { src: browser.icon_url }
            div {
                class: "px-2 w-full",

                Title { title: browser.title }
                Url { url: browser.url }
            }
        }
    }
}

#[component]
fn Icon(src: Memo<String>, #[props(default)] class: String) -> Element {
    let mut error = use_signal(|| false);
    let src = use_memo(move || {
        if src().is_empty() || error() {
            DEFAULT_ICON.to_string()
        } else {
            src()
        }
    });
    rsx! {
        div {
            class: "favicon avatar select-none {class}",

            div {
                class: "w-6 rounded",
                img {
                    src,
                    onerror: move |_| {
                        error.set(true);
                    },
                }
            }
        }
    }
}

#[component]
fn Title(title: Memo<String>) -> Element {
    rsx! {
        div {
            class: "title text-sm font-semibold truncate",
            "{title}"
        }
    }
}

#[component]
fn Url(url: Memo<String>) -> Element {
    let url = use_memo(move || {
        percent_decode_str(&url())
            .decode_utf8()
            .map(|u| u.to_string())
            .unwrap_or(url())
    });

    rsx! {
        div {
            class: "url text-xs text-blue-300 truncate",
            "{url}"
        }
    }
}
