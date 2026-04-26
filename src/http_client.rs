mod body;
mod header;
// mod log;
mod method;
mod response;
mod send;
mod uri;

use crate::{
    api::{HttpRequest, fetch},
    app::use_browser,
};
use body::BodyArea;
use dioxus::prelude::*;
use header::HeaderTable;
use method::MethodSelect;
use response::ResponseView;
use send::SendButton;
use uri::UriInput;

#[component]
pub fn HttpClientGate() -> Element {
    let mut is_client = use_browser().is_client;

    rsx! {
        div { class: "fab",
            label { class: "gate swap swap-rotate",
                input {
                    tabindex: "-1",
                    r#type: "checkbox",
                    onclick: move |_| is_client.toggle(),
                }

                div { class: "btn btn-primary btn-lg btn-circle swap-off", "🔗" }

                div { class: "btn btn-secondary btn-lg btn-circle swap-on", "X" }
            
            }
        }
    }
}

#[component]
pub fn HttpClient() -> Element {
    let method_value = use_signal(String::new);
    let uri_value = use_signal(String::new);
    let body_value = use_signal(String::new);
    let header_value = use_store(Vec::new);
    let body_editable = use_memo(move || {
        let method = method_value();
        "PATCH" == method || "POST" == method || "PUT" == method
    });

    let mut resp = use_action(move |req| async move { fetch(req).await });
    let pending = use_signal(|| false);

    let on_submit = move |_| {
        resp.call(HttpRequest::new(
            uri_value(),
            method_value(),
            header_value(),
            body_value(),
        ))
    };

    rsx! {
        div { class: "grid grid-cols-2 gap-4",
            div { class: "p-4 min-h-screen",
                div { class: "join join-vertical h-full w-full",
                    div { class: "join w-full join-item",
                        MethodSelect { value: method_value, class: "join-item" }
                        UriInput { value: uri_value, class: "w-full join-item" }
                        SendButton { onclick: on_submit, class: "join-item" }
                    }
                    div { class: "my-1" }
                    HeaderTable { rows: header_value, class: "w-full join-item" }
                    div { class: "my-1" }
                    BodyArea {
                        value: body_value,
                        contenteditable: body_editable(),
                        class: "h-full w-full join-item",
                    }
                }
            }
            if pending() {
                div { class: "flex min-h-screen",
                    div { class: "m-auto",
                        span { class: "loading loading-infinity loading-lg" }
                    }
                }
            } else {
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
                        None => rsx! {},
                    }
                }
            }
        }
    }
}
