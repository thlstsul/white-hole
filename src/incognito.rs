use dioxus::prelude::*;

use crate::{api::incognito, app::use_browser};

#[component]
pub fn Incognito(#[props(default)] class: String) -> Element {
    rsx! {
        li {
            label { class: "btn btn-ghost btn-block swap {class}",
                input {
                    r#type: "checkbox",
                    class: "theme-controller",
                    value: "synthwave",
                    checked: use_browser().incognito,
                    onclick: |_| async { incognito().await },
                }

                div { class: "swap-on", "🌑 无痕模式" }

                div { class: "swap-off", "🌕 无痕模式" }
            }
        }
    }
}
