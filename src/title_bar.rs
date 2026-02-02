use crate::{
    api::{focus, start_dragging},
    app::use_browser,
    darkreader::Darkreader,
    extension::Extension,
    navigation::Navigator,
    url::DecodeUrl,
    window_decoration::WindowDecoration,
};
use dioxus::{html::input_data::MouseButton, prelude::*};

const DEFAULT_ICON: Asset = asset!("/assets/default_icon.svg");

#[component]
pub fn TitleBar(#[props(default)] class: String) -> Element {
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
            class: "title-bar navbar min-h-10 h-10 {class}",
            onmousedown: start_dragging,

            Navigator { class: "flex-none" }
            TitleBarContent {}
            div { class: "fixed top-0 right-0 join",
                Extension { class: "join-item",
                    Darkreader { class: "tab" }
                }
                WindowDecoration { class: "join-item" }
            }
        }
    }
}

#[component]
fn TitleBarContent() -> Element {
    rsx! {
        div {
            class: "title-bar-content flex flex-row items-center max-w-2/3 group",
            onclick: |_| async { focus().await },
            onmousedown: |e| e.stop_propagation(),

            Icon {}
            div { class: "px-2 flex flex-col w-full",
                Title {}
                Url {}
            }

            div { class: "hidden group-hover:block w-full justify-center",
                kbd { class: "kbd kbd-sm", "CTRL" }
                kbd { class: "kbd kbd-sm", "L" }
            }
        }
    }
}

#[component]
fn Icon() -> Element {
    let src = use_browser().icon_url;

    let mut src = use_memo(move || {
        if src.is_empty() {
            DEFAULT_ICON.to_string()
        } else {
            src()
        }
    });

    rsx! {
        div { class: "favicon avatar select-none",
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
fn Title() -> Element {
    rsx! {
        div { class: "title text-sm font-semibold truncate", {use_browser().title} }
    }
}

#[component]
fn Url() -> Element {
    rsx! {
        div { class: "url text-xs text-blue-300 truncate",
            DecodeUrl { url: use_browser().url }
        }
    }
}
