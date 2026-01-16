use dioxus::prelude::*;

use crate::darkreader::Darkreader;

#[component]
pub fn Extension(#[props(default)] class: String) -> Element {
    let is_open = use_signal(|| false);

    rsx! {
        div {
            class: "extension join {class}",
            onmousedown: |e| e.stop_propagation(),

            Switcher { class: "mx-3 join-item", is_open }
            if is_open() {
                Darkreader { class: "mx-3 join-item" }
            }
        }
    }
}

#[component]
fn Switcher(#[props(default)] class: String, is_open: Signal<bool>) -> Element {
    rsx! {
        label { class: "switcher swap swap-rotate {class}",
            input {
                tabindex: "-1",
                r#type: "checkbox",
                checked: is_open,
                onclick: move |_| is_open.set(!is_open()),
            }

            svg {
                class: "swap-off fill-current size-5",
                view_box: "0 0 512 512",
                xmlns: "http://www.w3.org/2000/svg",
                path { d: "M64,384H448V341.33H64Zm0-106.67H448V234.67H64ZM64,128v42.67H448V128Z" }
            }
            svg {
                class: "swap-on fill-current size-5",
                view_box: "0 0 512 512",
                xmlns: "http://www.w3.org/2000/svg",
                polygon { points: "400 145.49 366.51 112 256 222.51 145.49 112 112 145.49 222.51 256 112 366.51 145.49 400 256 289.49 366.51 400 400 366.51 289.49 256 400 145.49" }
            }
        }
    }
}
