use dioxus::prelude::*;

use crate::app::Browser;

const MODE_DARK_ICON: Asset = asset!("/assets/mode-dark.svg");
const MODE_LIGHT_ICON: Asset = asset!("/assets/mode-light.svg");

#[component]
pub fn Darkreader(#[props(default)] class: String) -> Element {
    let darkreader = use_context::<Browser>().darkreader;

    rsx! {
        label { class: "darkreader swap swap-flip {class}",
            input {
                tabindex: "-1",
                r#type: "checkbox",
                checked: darkreader,
                onclick: |_| async { crate::api::darkreader().await },
            }

            div { class: "swap-on w-6 rounded",
                img { src: MODE_DARK_ICON }
            }
            div { class: "swap-off w-6 rounded",
                img { src: MODE_LIGHT_ICON }
            }
        }
    }
}
