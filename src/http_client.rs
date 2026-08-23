mod body;
mod header;
mod method;
mod response;
mod send;
mod uri;

use crate::api::{HttpRequest, fetch, http_client};
use body::BodyArea;
use dioxus::prelude::*;
use header::HeaderTable;
use method::MethodSelect;
use response::ResponseView;
use send::SendButton;
use uri::UriInput;

#[component]
pub fn HttpClient() -> Element {
    let method_value = use_signal(|| String::from("POST"));
    let uri_value = use_signal(String::new);
    let body_value = use_signal(String::new);
    let header_value = use_store(Vec::new);
    let body_editable = use_memo(move || {
        let method = method_value();
        "PATCH" == method || "POST" == method || "PUT" == method
    });
    let mut begin = use_signal(|| false);

    let mut resp = use_action(move |req| async move { fetch(req).await });

    let on_submit = move |_| {
        begin.set(true);
        resp.call(HttpRequest::new(
            uri_value(),
            method_value(),
            header_value(),
            body_value(),
        ))
    };

    rsx! {
        div { class: "fixed top-0 right-0", Close {} }

        div { class: "grid grid-cols-2 gap-4",
            div { class: "p-4 min-h-screen",
                div { class: "join join-vertical h-full w-full",
                    div { class: "join w-full",
                        MethodSelect { value: method_value, class: "join-item" }
                        UriInput { value: uri_value, class: "w-full join-item" }
                        SendButton { onclick: on_submit, class: "join-item" }
                    }
                    div { class: "my-1" }
                    HeaderTable { rows: header_value, class: "w-full" }
                    div { class: "my-1" }
                    BodyArea {
                        value: body_value,
                        editable: body_editable(),
                        class: "h-full w-full",
                    }
                }
            }

            div { class: "p-4",
                match resp.value() {
                    Some(Ok(rr)) => rsx! {
                        ResponseView { resp: rr }
                    },
                    Some(Err(e)) => rsx! {
                        div { role: "alert", class: "alert alert-error",
                            svg {
                                xmlns: "http://www.w3.org/2000/svg",
                                class: "h-6 w-6 shrink-0 stroke-current",
                                fill: "none",
                                view_box: "0 0 24 24",
                                path {
                                    stroke_linecap: "round",
                                    stroke_linejoin: "round",
                                    stroke_width: "2",
                                    d: "M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z",
                                }
                            }
                            span { "{e}" }
                        }
                    },
                    None => rsx! {
                        if begin() {
                            div { class: "flex h-full",
                                div { class: "m-auto",
                                    span { class: "loading loading-infinity loading-lg" }
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}

#[component]
pub fn HttpClientGate(#[props(default)] class: String) -> Element {
    rsx! {
        button {
            class: "gate {class}",
            tabindex: "-1",
            onclick: |_| async { http_client().await },

            svg {
                xmlns: "http://www.w3.org/2000/svg",
                class: "size-5 shrink-0 stroke-current",
                fill: "none",
                view_box: "0 0 24 24",
                path {
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    stroke_width: "2",
                    d: "M12 21a9.004 9.004 0 0 0 8.716-6.747M12 21a9.004 9.004 0 0 1-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 0 1 7.843 4.582M12 3a8.997 8.997 0 0 0-7.843 4.582m15.686 0A11.953 11.953 0 0 1 12 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0 1 21 12c0 .778-.099 1.533-.284 2.253m0 0A17.919 17.919 0 0 1 12 16.5c-3.162 0-6.133-.815-8.716-2.247m0 0A9.015 9.015 0 0 1 3 12c0-1.605.42-3.113 1.157-4.418",
                }
            }
        }
    }
}

#[component]
fn Close(#[props(default)] class: String) -> Element {
    rsx! {
        button {
            tabindex: "-1",
            class: "window-close btn btn-square btn-ghost rounded-none {class}",
            onclick: |_| async { http_client().await },

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
