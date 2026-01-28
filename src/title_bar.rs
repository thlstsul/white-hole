use crate::{
    api::{focus, start_dragging},
    app::use_browser,
    extension::Extension,
    navigation::Navigator,
    url::percent_decode_str,
    window_decoration::WindowDecoration,
};
use dioxus::{html::input_data::MouseButton, prelude::*};

const DEFAULT_ICON: Asset = asset!("/assets/default_icon.svg");

#[component]
pub fn TitleBar() -> Element {
    let start_dragging = move |e: MouseEvent| async move {
        let Some(button) = e.trigger_button() else {
            return;
        };

        if button == MouseButton::Primary {
            start_dragging().await;
        }
    };

    rsx! {
        div {
            class: "title-bar navbar min-h-10 h-10",
            onmousedown: start_dragging,

            Navigator { class: "flex-none" }
            TitleBarContent {}
            div { class: "fixed top-0 right-0 join",
                Extension { class: "join-item" }
                WindowDecoration { class: "join-item" }
            }
        }
    }
}

#[component]
fn TitleBarContent(#[props(default)] class: String) -> Element {
    let browser = use_browser();

    rsx! {
        div {
            class: "title-bar-content flex flex-row items-center max-w-2/3 group {class}",
            onclick: |_| async { focus().await },
            onmousedown: |e| e.stop_propagation(),

            Icon { src: browser.icon_url }
            div { class: "px-2 flex flex-col w-full",
                Title { title: browser.title }
                Url { url: browser.url }
            }

            div { class: "hidden group-hover:block w-full justify-center",
                kbd { class: "kbd kbd-sm", "CTRL" }
                kbd { class: "kbd kbd-sm", "L" }
            }
        }
    }
}

#[component]
fn Icon(src: ReadSignal<String>, #[props(default)] class: String) -> Element {
    let mut src = use_memo(move || {
        if src().is_empty() {
            DEFAULT_ICON.to_string()
        } else {
            src()
        }
    });

    rsx! {
        div { class: "favicon avatar select-none {class}",
            div { class: "w-6 rounded",
                img {
                    src,
                    onerror: move |_| {
                        src.set(DEFAULT_ICON.to_string());
                    },
                }
            }
        }
    }
}

#[component]
fn Title(title: ReadSignal<String>) -> Element {
    rsx! {
        div { class: "title text-sm font-semibold truncate", "{title}" }
    }
}

#[component]
fn Url(url: ReadSignal<String>) -> Element {
    let url = use_memo(move || {
        percent_decode_str(&url())
            .decode_utf8()
            .map(|u| u.to_string())
            .unwrap_or(url())
    });

    rsx! {
        div { class: "url text-xs text-blue-300 truncate", "{url}" }
    }
}
