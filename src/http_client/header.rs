use dioxus::prelude::*;

use crate::api::{HttpHeader, HttpHeaderStoreExt};

#[component]
pub fn HeaderTable(#[props(default)] class: String, rows: Store<Vec<HttpHeader>>) -> Element {
    let mut header_name = use_store(String::new);
    let mut header_value = use_store(String::new);

    rsx! {
        table { class: "header-table {class}",
            tbody {
                td { class: "w-1/3",
                    HeaderNameInput { value: header_name }
                }
                td {
                    HeaderValueInput { value: header_value }
                }
                td { class: "h-6 w-6",
                    button {
                        class: "header-add btn btn-ghost",
                        onclick: move |_| {
                            rows.write()
                                .push(HttpHeader {
                                    key: header_name(),
                                    value: header_value(),
                                });
                            header_name.clear();
                            header_value.clear();
                        },

                        svg {
                            xmlns: "http://www.w3.org/2000/svg",
                            view_box: "0 0 24 24",
                            fill: "currentColor",
                            class: "size-6",
                            path {
                                fill_rule: "evenodd",
                                d: "M19.916 4.626a.75.75 0 0 1 .208 1.04l-9 13.5a.75.75 0 0 1-1.154.114l-6-6a.75.75 0 0 1 1.06-1.06l5.353 5.353 8.493-12.739a.75.75 0 0 1 1.04-.208Z",
                                clip_rule: "evenodd",
                            }
                        }
                    }
                }
                for (i, item) in rows.iter().enumerate() {
                    tr {
                        td { class: "w-1/3",
                            HeaderNameInput { value: item.key() }
                        }
                        td {
                            HeaderValueInput { value: item.value() }
                        }
                        td { class: "h-6 w-6",
                            button {
                                class: "header-delete btn btn-ghost",
                                onclick: move |_| {
                                    rows.write().remove(i);
                                },

                                svg {
                                    xmlns: "http://www.w3.org/2000/svg",
                                    view_box: "0 0 24 24",
                                    fill: "currentColor",
                                    class: "size-6",
                                    path {
                                        fill_rule: "evenodd",
                                        d: "M5.47 5.47a.75.75 0 0 1 1.06 0L12 10.94l5.47-5.47a.75.75 0 1 1 1.06 1.06L13.06 12l5.47 5.47a.75.75 0 1 1-1.06 1.06L12 13.06l-5.47 5.47a.75.75 0 0 1-1.06-1.06L10.94 12 5.47 6.53a.75.75 0 0 1 0-1.06Z",
                                        clip_rule: "evenodd",
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn HeaderNameInput(#[props(default)] class: String, value: Store<String>) -> Element {
    rsx! {
        div { class: "header-name dropdown w-full {class}",
            input {
                value,
                tabindex: "0",
                class: "input input-ghost input-neutral w-full",
                placeholder: "Header name",
                oninput: move |ev| {
                    value.set(ev.value());
                },
            }

            ul {
                tabindex: "0",
                class: "dropdown-content menu z-1 shadow bg-base-100 rounded-box w-full p-2",
                for name in HEADERS {
                    if value().is_empty() || name.starts_with(&value()) {
                        li {
                            a {
                                onclick: move |_| {
                                    value.set(name.to_string());
                                },

                                "{name}"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn HeaderValueInput(#[props(default)] class: String, value: Store<String>) -> Element {
    rsx! {
        input {
            value,
            class: "header-value input input-ghost input-neutral w-full {class}",
            r#type: "text",
            placeholder: "value",
            oninput: move |e| {
                value.set(e.value());
            },
        }
    }
}

const HEADERS: &[&str] = &[
    "accept",
    "accept-charset",
    "accept-encoding",
    "accept-language",
    "accept-ranges",
    "access-control-allow-credentials",
    "access-control-allow-headers",
    "access-control-allow-methods",
    "access-control-allow-origin",
    "access-control-expose-headers",
    "access-control-max-age",
    "access-control-request-headers",
    "access-control-request-method",
    "age",
    "allow",
    "alt-svc",
    "authorization",
    "cache-control",
    "cache-status",
    "cdn-cache-control",
    "connection",
    "content-disposition",
    "content-encoding",
    "content-language",
    "content-length",
    "content-location",
    "content-range",
    "content-security-policy",
    "content-security-policy-report-only",
    "content-type",
    "cookie",
    "date",
    "dnt",
    "etag",
    "expect",
    "expires",
    "forwarded",
    "from",
    "host",
    "if-match",
    "if-modified-since",
    "if-none-match",
    "if-range",
    "if-unmodified-since",
    "last-modified",
    "link",
    "location",
    "max-forwards",
    "origin",
    "pragma",
    "proxy-authenticate",
    "proxy-authorization",
    "public-key-pins",
    "public-key-pins-report-only",
    "range",
    "referer",
    "referrer-policy",
    "refresh",
    "retry-after",
    "sec-websocket-accept",
    "sec-websocket-extensions",
    "sec-websocket-key",
    "sec-websocket-protocol",
    "sec-websocket-version",
    "server",
    "set-cookie",
    "strict-transport-security",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "upgrade-insecure-requests",
    "user-agent",
    "vary",
    "via",
    "warning",
    "www-authenticate",
    "x-content-type-options",
    "x-dns-prefetch-control",
    "x-frame-options",
    "x-xss-protection",
];
