use crate::{
    api::{focus, start_dragging},
    app::use_browser,
    darkreader::Darkreader,
    extension::Extension,
    http_client::HttpClientGate,
    incognito::Incognito,
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
            class: "title-bar navbar pl-0 pr-0 min-h-10 h-10 {class}",
            onmousedown: start_dragging,

            Navigator { class: "flex-none" }
            TitleBarContent {}
            div { class: "flex-none ml-auto join",
                Extension {
                    Incognito { class: "tab" }
                    HttpClientGate { class: "tab" }
                    Darkreader { class: "tab" }
                }
                WindowDecoration {}
            }
        }
    }
}

#[component]
fn TitleBarContent() -> Element {
    rsx! {
        div { class: "title-bar-content relative flex flex-row items-center flex-1 min-w-0 group",
            // 左半：点击触发 focus 并拦截 mousedown 阻止拖动；右半无拦截，冒泡到 title-bar 拖动
            div {
                class: "absolute left-0 top-0 w-1/2 h-full",
                onclick: |_| async { focus().await },
                onmousedown: |e| e.stop_propagation(),
            }

            Icon {}
            div { class: "px-2 flex flex-col w-full",
                Title {}
                Url {}
            }
        }
    }
}

#[component]
fn Icon() -> Element {
    let icon_url = use_browser().icon_url;

    let mut src = use_memo(move || {
        let url = icon_url();
        if url.is_empty() {
            DEFAULT_ICON.to_string()
        } else {
            url
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
