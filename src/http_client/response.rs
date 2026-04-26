use std::{collections::HashMap, str::FromStr, string::FromUtf8Error};

use crate::api::{HttpHeader, HttpResponse};
use dioxus::{logger::tracing::error, prelude::*};
use encoding::{
    DecoderTrap, Encoding,
    all::{ASCII, GB18030, GBK, ISO_8859_1, UTF_8},
};
use http::StatusCode;
use mime::{CHARSET, Mime};
use serde_json::Value;
use time::{OffsetDateTime, macros::format_description};

#[component]
pub fn ResponseView(resp: ReadSignal<HttpResponse>) -> Element {
    let HttpResponse {
        done_date,
        status,
        headers,
        body,
        elapsed_time,
    } = resp();

    let mut header_map = HashMap::new();
    for HttpHeader { key, value } in headers.into_iter() {
        header_map.insert(key, value);
    }

    let status = StatusCode::try_from(status).unwrap_or_default();

    let content_type = header_map
        .get("content-type")
        .map(|c| Mime::from_str(c))
        .transpose()
        .unwrap_or(None);

    rsx! {
        Stat { status, elapsed_time, done_date }
        div { class: "divider h-0" }
        Header { header: header_map }
        div { class: "divider h-0" }
        Body { content_type, body }
    }
}

#[component]
fn Body(body: Vec<u8>, content_type: Option<Mime>) -> Element {
    let body = if let Some(content_type) = content_type {
        let base = content_type.type_();
        let sub = content_type.subtype().as_str();
        if "text" == base
            || matches!(sub, "json" | "x-www-form-urlencoded" | "markdown" | "rtf")
            || sub.contains("xml")
        {
            let body = if "json" == sub {
                serde_json::from_slice::<Value>(&body)
                    .and_then(|body| serde_json::to_vec_pretty(&body))
                    .unwrap_or(body)
            } else {
                body
            };

            let charset = content_type
                .get_param(CHARSET)
                .unwrap_or(mime::UTF_8)
                .as_str();

            decode(body, charset)?
        } else {
            format!("{body:?}")
        }
    } else {
        decode(body, "utf-8")?
    };

    // TODO raw body
    rsx! {
        pre { class: "p-4 rounded-md w-full overflow-x-auto",
            code { {body} }
        }
    }
}

#[component]
fn Header(header: HashMap<String, String>) -> Element {
    rsx! {
        div { class: "collapse collapse-arrow",
            input { r#type: "checkbox" }
            div { class: "collapse-title text-xl font-medium", "Headers" }
            div { class: "collapse-content",
                table { class: "table table-xs w-full",
                    tbody {
                        for (key , value) in header {
                            tr {
                                td { "{key}" }
                                td { "{value}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Stat(status: StatusCode, elapsed_time: i32, done_date: OffsetDateTime) -> Element {
    let (color_class, status_icon) = if status.is_success() {
        (
            "text-success",
            "M2.25 12c0-5.385 4.365-9.75 9.75-9.75s9.75 4.365 9.75 9.75-4.365 9.75-9.75 9.75S2.25 17.385 2.25 12Zm13.36-1.814a.75.75 0 1 0-1.22-.872l-3.236 4.53L9.53 12.22a.75.75 0 0 0-1.06 1.06l2.25 2.25a.75.75 0 0 0 1.14-.094l3.75-5.25Z",
        )
    } else if status.is_client_error() || status.is_server_error() {
        (
            "text-error",
            "M12 2.25c-5.385 0-9.75 4.365-9.75 9.75s4.365 9.75 9.75 9.75 9.75-4.365 9.75-9.75S17.385 2.25 12 2.25Zm-1.72 6.97a.75.75 0 1 0-1.06 1.06L10.94 12l-1.72 1.72a.75.75 0 1 0 1.06 1.06L12 13.06l1.72 1.72a.75.75 0 1 0 1.06-1.06L13.06 12l1.72-1.72a.75.75 0 1 0-1.06-1.06L12 10.94l-1.72-1.72Z",
        )
    } else {
        (
            "text-warning",
            "M2.25 12c0-5.385 4.365-9.75 9.75-9.75s9.75 4.365 9.75 9.75-4.365 9.75-9.75 9.75S2.25 17.385 2.25 12Zm8.706-1.442c1.146-.573 2.437.463 2.126 1.706l-.709 2.836.042-.02a.75.75 0 0 1 .67 1.34l-.04.022c-1.147.573-2.438-.463-2.127-1.706l.71-2.836-.042.02a.75.75 0 1 1-.671-1.34l.041-.022ZM12 9a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Z",
        )
    };

    rsx! {
        div { class: "stats shadow w-full",
            div { class: "stat py-0",
                div { class: format!("stat-figure {}", color_class),
                    svg {
                        xmlns: "http://www.w3.org/2000/svg",
                        fill: "currentColor",
                        view_box: "0 0 24 24",
                        class: "h-5 w-5",
                        path {
                            fill_rule: "evenodd",
                            clip_rule: "evenodd",
                            d: status_icon,
                        }
                    }
                }
                div { class: "stat-title", "Status" }
                div { class: format!("stat-value text-2xl {}", color_class), "{status}" }
                div { class: "stat-desc", {status.canonical_reason()} }
            }

            div { class: "stat py-0",
                div { class: "stat-figure",
                    svg {
                        xmlns: "http://www.w3.org/2000/svg",
                        fill: "currentColor",
                        view_box: "0 0 24 24",
                        class: "h-5 w-5",
                        path {
                            fill_rule: "evenodd",
                            clip_rule: "evenodd",
                            d: "M12 2.25c-5.385 0-9.75 4.365-9.75 9.75s4.365 9.75 9.75 9.75 9.75-4.365 9.75-9.75S17.385 2.25 12 2.25ZM12.75 6a.75.75 0 0 0-1.5 0v6c0 .414.336.75.75.75h4.5a.75.75 0 0 0 0-1.5h-3.75V6Z",
                        }
                    }
                }
                div { class: "stat-title", "Elapsed" }
                div { class: "stat-value text-2xl", "{elapsed_time}" }
                div { class: "stat-desc", "ms" }
            }

            div { class: "stat py-0",
                div { class: "stat-figure",
                    svg {
                        xmlns: "http://www.w3.org/2000/svg",
                        fill: "currentColor",
                        view_box: "0 0 24 24",
                        class: "h-5 w-5",
                        path { d: "M12.75 12.75a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0ZM7.5 15.75a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5ZM8.25 17.25a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0ZM9.75 15.75a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5ZM10.5 17.25a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0ZM12 15.75a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5ZM12.75 17.25a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0ZM14.25 15.75a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5ZM15 17.25a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0ZM16.5 15.75a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5ZM15 12.75a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0ZM16.5 13.5a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Z" }
                        path {
                            fill_rule: "evenodd",
                            clip_rule: "evenodd",
                            d: "M6.75 2.25A.75.75 0 0 1 7.5 3v1.5h9V3A.75.75 0 0 1 18 3v1.5h.75a3 3 0 0 1 3 3v11.25a3 3 0 0 1-3 3H5.25a3 3 0 0 1-3-3V7.5a3 3 0 0 1 3-3H6V3a.75.75 0 0 1 .75-.75Zm13.5 9a1.5 1.5 0 0 0-1.5-1.5H5.25a1.5 1.5 0 0 0-1.5 1.5v7.5a1.5 1.5 0 0 0 1.5 1.5h13.5a1.5 1.5 0 0 0 1.5-1.5v-7.5Z",
                        }
                    }
                }
                div { class: "stat-title", "At" }
                div { class: "stat-value text-2xl",
                    {
                        done_date
                            .format(format_description!("[hour]:[minute]:[second]"))
                            .unwrap_or_default()
                    }
                }
                div { class: "stat-desc",
                    {
                        done_date.format(format_description!("[year]-[month]-[day]")).unwrap_or_default()
                    }
                }
            }
        }
    }
}

fn decode(text: Vec<u8>, charset: &str) -> Result<String, FromUtf8Error> {
    const TRAP: DecoderTrap = DecoderTrap::Strict;
    let result = match charset {
        "ascii" => ASCII.decode(&text, TRAP),
        "gb18030" => GB18030.decode(&text, TRAP),
        "gbk" => GBK.decode(&text, TRAP),
        "iso-8859-1" => ISO_8859_1.decode(&text, TRAP),
        _ => UTF_8.decode(&text, TRAP),
    };

    result
        .inspect_err(|e| error!("{e}"))
        .or(String::from_utf8(text))
}
