use std::rc::Rc;

use dioxus::prelude::*;
use time::{OffsetDateTime, macros::format_description};

use crate::{
    api::{PageToken, open_tab, query_navigation_log, update_star},
    app::Browser,
    search_input::SearchInput,
    settings::Settings,
};

const DEFAULT_ICON: Asset = asset!("/assets/default_icon.svg");

#[component]
pub fn SearchPage() -> Element {
    let mut keyword = use_signal(String::new);
    let mut page_token = use_signal(PageToken::default);
    let mut next_page_token = use_signal(|| None);
    let mut main_element = use_signal(|| None);
    let mut logs = use_signal(Vec::new);
    let mut focus_log = use_signal(|| None);
    let input_element = use_signal::<Option<Rc<MountedData>>>(|| None);

    use_effect(move || {
        // 输入关键字进行检索、切换模式时，重置页码
        let _ = (keyword.read(), use_context::<Browser>().incognito.read());
        page_token.set(PageToken::default());
        next_page_token.set(None);
    });

    let _ = use_resource(move || async move {
        let Ok(response) = query_navigation_log(keyword(), page_token()).await else {
            return;
        };

        if page_token() == PageToken::default() {
            logs.clear();
        }

        next_page_token.set(response.next_page_token);
        logs.extend(response.logs);
    });

    rsx! {
        div {
            class: "max-h-screen flex flex-col",
            onkeydown: move |e| async move {
                if e.key() == Key::Enter {
                    if let Some(focus_log) = focus_log() {
                        let _ = open_tab(focus_log.id).await;
                    }
                } else if e.key() == Key::ArrowRight {
                    if let Some(log) = focus_log() {
                        e.prevent_default();
                        keyword.set(log.url.clone());
                        if let Some(url) = input_element() {
                            let _ = url.set_focus(true).await;
                            focus_log.set(None);
                        }
                    }
                }
            },

            header {
                div { class: "w-full join",
                    SearchInput { class: "join-item", keyword, input_element }
                    Settings { class: "join-item" }
                }
            }
            main {
                class: "flex-1 overflow-auto",
                onmounted: move |element| main_element.set(Some(element.data())),
                onscroll: move |_| async move {
                    let Some(main_element) = main_element() else {
                        return;
                    };

                    let (Ok(size), Ok(offset), Ok(client)) = (

                        main_element.get_scroll_size().await,
                        main_element.get_scroll_offset().await,
                        main_element.get_client_rect().await,
                    ) else {
                        return;
                    };
                    if size.height - offset.y - client.size.height < 10.
                        && let Some(next_page_token) = next_page_token()
                    {
                        page_token.set(next_page_token);
                    }
                },

                ul { class: "list",

                    for log in logs() {
                        li {
                            tabindex: "0",
                            key: "{log.id}",
                            class: "list-row",
                            onfocus: move |_| {
                                focus_log
                                    .set(
                                        Some(FocusLog {
                                            id: log.id,
                                            url: log.url.clone(),
                                        }),
                                    )
                            },
                            onclick: move |_| async move {
                                let _ = open_tab(log.id).await;
                            },

                            Icon { url: log.icon_url.clone() }
                            div { class: "list-col-grow",

                                div { "{log.title}" }
                                div { class: "text-xs opacity-60", "{log.url}" }
                            }

                            if let Some(last_time) = log.last_time {
                                LogTime { last_time }
                            }

                            Star { checked: log.star, log_id: log.id }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Default, PartialEq)]
struct FocusLog {
    id: i64,
    url: String,
}

#[component]
fn Icon(url: String) -> Element {
    let mut url = use_signal(move || url);

    rsx! {
        div {
            img {
                class: "size-9",
                src: "{url}",
                onerror: move |_| url.set(DEFAULT_ICON.to_string()),
            }
        }
    }
}

#[component]
fn LogTime(last_time: OffsetDateTime) -> Element {
    let last_time =
        last_time.format(format_description!("[year]-[month]-[day] [hour]:[minute]"))?;
    rsx! {
        div { class: "text-xs opacity-60", "{last_time}" }
    }
}

#[component]
fn Star(log_id: i64, checked: bool) -> Element {
    let mut checked = use_signal(use_reactive!(|checked| checked));

    rsx! {
        label { class: "swap", onclick: |e| e.stop_propagation(),

            input {
                tabindex: "-1",
                r#type: "checkbox",
                checked,
                onchange: move |_| async move {
                    checked.set(!checked());
                    let _ = update_star(log_id).await;
                },
            }

            svg {
                xmlns: "http://www.w3.org/2000/svg",
                fill: "currentColor",
                view_box: "0 0 24 24",
                stroke_width: "1.5",
                stroke: "currentColor",
                class: "size-6 swap-on",
                path {
                    fill_rule: "evenodd",
                    d: "M10.788 3.21c.448-1.077 1.976-1.077 2.424 0l2.082 5.006 5.404.434c1.164.093 1.636 1.545.749 2.305l-4.117 3.527 1.257 5.273c.271 1.136-.964 2.033-1.96 1.425L12 18.354 7.373 21.18c-.996.608-2.231-.29-1.96-1.425l1.257-5.273-4.117-3.527c-.887-.76-.415-2.212.749-2.305l5.404-.434 2.082-5.005Z",
                    clip_rule: "evenodd",
                }
            }

            svg {
                xmlns: "http://www.w3.org/2000/svg",
                fill: "none",
                view_box: "0 0 24 24",
                stroke_width: "1.5",
                stroke: "currentColor",
                class: "size-6 swap-off",
                path {
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    d: "M11.48 3.499a.562.562 0 0 1 1.04 0l2.125 5.111a.563.563 0 0 0 .475.345l5.518.442c.499.04.701.663.321.988l-4.204 3.602a.563.563 0 0 0-.182.557l1.285 5.385a.562.562 0 0 1-.84.61l-4.725-2.885a.562.562 0 0 0-.586 0L6.982 20.54a.562.562 0 0 1-.84-.61l1.285-5.386a.562.562 0 0 0-.182-.557l-4.204-3.602a.562.562 0 0 1 .321-.988l5.518-.442a.563.563 0 0 0 .475-.345L11.48 3.5Z",
                }
            }
        }
    }
}

#[component]
fn LogLoding() -> Element {
    rsx! {
        div { class: "flex flex-col gap-4 w-full h-full overflow-hidden",
            for _ in 0..10 {
                div { class: "skeleton h-16 w-full" }
            }
        }
    }
}
