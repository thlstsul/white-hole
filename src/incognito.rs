use dioxus::prelude::*;

use crate::{api::incognito, app::use_browser};

#[component]
pub fn Incognito(#[props(default)] class: String) -> Element {
    rsx! {
        li {
            label { class: "incognito swap swap-rotate {class}",
                input {
                    r#type: "checkbox",
                    class: "theme-controller",
                    value: "synthwave",
                    checked: use_browser().incognito,
                    onclick: |_| async { incognito().await },
                }

                div { class: "btn btn-lg btn-circle swap-on", "🌑" }

                div { class: "btn btn-lg btn-circle bg-white swap-off", "🌕" }
            }
        }
    }
}
