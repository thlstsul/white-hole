use dioxus::prelude::*;

use crate::app::use_browser;

#[component]
pub fn Incognito(#[props(default)] class: String) -> Element {
    let incognito = use_browser().incognito;

    rsx! {
        li {
            label { class: "btn btn-ghost btn-block swap {class}",
                input {
                    r#type: "checkbox",
                    class: "theme-controller",
                    value: "synthwave",
                    checked: incognito,
                    onclick: |_| async { crate::api::incognito().await },
                }

                div { class: "swap-on", "🌑 无痕模式" }

                div { class: "swap-off", "🌕 无痕模式" }
            }
        }
    }
}
