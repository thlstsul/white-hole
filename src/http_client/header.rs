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
                                d: "M12 2.25c-5.385 0-9.75 4.365-9.75 9.75s4.365 9.75 9.75 9.75 9.75-4.365 9.75-9.75S17.385 2.25 12 2.25ZM12.75 9a.75.75 0 0 0-1.5 0v2.25H9a.75.75 0 0 0 0 1.5h2.25V15a.75.75 0 0 0 1.5 0v-2.25H15a.75.75 0 0 0 0-1.5h-2.25V9Z",
                                clip_rule: "evenodd",
                            }
                        }
                    }
                }
                for (i , item) in rows.iter().enumerate() {
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
                                        d: "M16.5 4.478v.227a48.816 48.816 0 0 1 3.878.512.75.75 0 1 1-.256 1.478l-.209-.035-1.005 13.07a3 3 0 0 1-2.991 2.77H8.084a3 3 0 0 1-2.991-2.77L4.087 6.66l-.209.035a.75.75 0 0 1-.256-1.478A48.567 48.567 0 0 1 7.5 4.705v-.227c0-1.564 1.213-2.9 2.816-2.951a52.662 52.662 0 0 1 3.369 0c1.603.051 2.815 1.387 2.815 2.951Zm-6.136-1.452a51.196 51.196 0 0 1 3.273 0C14.39 3.05 15 3.684 15 4.478v.113a49.488 49.488 0 0 0-6 0v-.113c0-.794.609-1.428 1.364-1.452Zm-.355 5.945a.75.75 0 1 0-1.5.058l.347 9a.75.75 0 1 0 1.499-.058l-.346-9Zm5.48.058a.75.75 0 1 0-1.498-.058l-.347 9a.75.75 0 0 0 1.5.058l.345-9Z",
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
                class: "dropdown-content menu z-[1] shadow bg-base-100 rounded-box w-full p-2",
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
