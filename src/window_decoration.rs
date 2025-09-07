use dioxus::prelude::*;
use tauri_sys::core::invoke;

use crate::app::Browser;

#[component]
pub fn WindowDecoration(#[props(default)] class: String) -> Element {
    rsx! {
        div {
            class: "window-decoration join {class}",
            onmousedown: |e| e.stop_propagation(),

            Minimize { class: "join-item" }
            MaximizeOr { class: "join-item" }
            Close { class: "join-item" }
        }
    }
}

#[component]
fn Minimize(#[props(default)] class: String) -> Element {
    let minimize = |_| async move {
        invoke::<()>("minimize", &()).await;
    };

    rsx! {
        button {
            class: "window-minimize btn btn-square btn-ghost {class}",
            id: "window-minimize",
            onclick: minimize,
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                class: "size-4",
                view_box: "0 0 24 24",
                path {
                    fill: "currentColor",
                    d: "M20 14H4v-4h16",
                }
            }
        }
    }
}

#[component]
fn Close(#[props(default)] class: String) -> Element {
    let close = |_| async move {
        invoke::<()>("close", &()).await;
    };

    rsx! {
        button {
            class: "window-close btn btn-square btn-ghost btn-secondary {class}",
            id: "window-close",
            onclick: close,
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                class: "size-5",
                view_box: "0 0 24 24",
                path {
                    fill: "currentColor",
                    d: "M19 6.41L17.59 5L12 10.59L6.41 5L5 6.41L10.59 12L5 17.59L6.41 19L12 13.41L17.59 19L19 17.59L13.41 12z",
                }
            }
        }
    }
}

#[component]
fn MaximizeOr(#[props(default)] class: String) -> Element {
    let maximized = use_context::<Browser>().maximized;

    rsx! {
        if maximized() {
            Unmaximize { class: "join-item" }
        } else {
            Maximize { class: "join-item" }
        }
    }
}

#[component]
fn Maximize(#[props(default)] class: String) -> Element {
    let maximize = move |_| async move {
        invoke::<()>("maximize", &()).await;
    };

    rsx! {
        button {
            class: "window-maximize btn btn-square btn-ghost btn-primary {class}",
            id: "window-maximize",
            onclick: maximize,
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                class: "size-4",
                view_box: "0 0 24 24",
                path {
                    fill: "currentColor",
                    d: "M4 4h16v16H4zm2 4v10h12V8z",
                }
            }
        }
    }
}

#[component]
fn Unmaximize(#[props(default)] class: String) -> Element {
    let unmaximize = move |_| async move {
        invoke::<()>("unmaximize", &()).await;
    };

    rsx! {
        button {
            class: "window-unmaximize btn btn-square btn-ghost btn-primary {class}",
            id: "window-unmaximize",
            onclick: unmaximize,
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                class: "size-4",
                view_box: "0 0 24 24",
                path {
                    fill: "currentColor",
                    d: "M4 8h4V4h12v12h-4v4H4zm12 0v6h2V6h-8v2zM6 12v6h8v-6z"
                }
            }
        }
    }
}
