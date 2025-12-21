use dioxus::prelude::*;

use crate::{
    api::{back, forward, reload},
    app::Browser,
};

#[component]
pub fn Navigator(#[props(default)] class: String) -> Element {
    rsx! {
        div {
            class: "navigator join {class}",
            onmousedown: |e| e.stop_propagation(),

            Back { class: "join-item" }
            ReloadOr { class: "join-item" }
            Forward { class: "join-item" }
        }
    }
}

#[component]
fn Back(#[props(default)] class: String) -> Element {
    let can_back = use_context::<Browser>().can_back;

    rsx! {
        button {
            tabindex: "-1",
            class: "btn btn-square btn-ghost join-item",
            disabled: !can_back(),
            onclick: |_| async { back().await },

            svg {
                fill: "currentColor",
                class: "size-5",
                view_box: "0 0 24 24",
                xmlns: "http://www.w3.org/2000/svg",
                path { d: "M17.921,1.505a1.5,1.5,0,0,1-.44,1.06L9.809,10.237a2.5,2.5,0,0,0,0,3.536l7.662,7.662a1.5,1.5,0,0,1-2.121,2.121L7.688,15.9a5.506,5.506,0,0,1,0-7.779L15.36.444a1.5,1.5,0,0,1,2.561,1.061Z" }
            }
        }
    }
}

#[component]
fn Forward(#[props(default)] class: String) -> Element {
    let can_forward = use_context::<Browser>().can_forward;

    rsx! {
        button {
            tabindex: "-1",
            class: "forward btn btn-square btn-ghost {class}",
            disabled: !can_forward(),
            onclick: |_| async { forward().await },

            svg {
                fill: "currentColor",
                class: "size-5",
                view_box: "0 0 24 24",
                xmlns: "http://www.w3.org/2000/svg",
                path { d: "M6.079,22.5a1.5,1.5,0,0,1,.44-1.06l7.672-7.672a2.5,2.5,0,0,0,0-3.536L6.529,2.565A1.5,1.5,0,0,1,8.65.444l7.662,7.661a5.506,5.506,0,0,1,0,7.779L8.64,23.556A1.5,1.5,0,0,1,6.079,22.5Z" }
            }
        }
    }
}

#[component]
fn ReloadOr(#[props(default)] class: String) -> Element {
    let loading = use_context::<Browser>().loading;

    rsx! {
        if loading() {
            Loading { class: "join-item" }
        } else {
            Reload { class: "join-item" }
        }
    }
}

#[component]
fn Reload(#[props(default)] class: String) -> Element {
    rsx! {
        button {
            tabindex: "-1",
            class: "go btn btn-square btn-ghost {class}",
            onclick: |_| async { reload().await },

            svg {
                fill: "currentColor",
                class: "size-5",
                view_box: "0 0 24 24",
                xmlns: "http://www.w3.org/2000/svg",
                path { d: "m12,0C5.383,0,0,5.383,0,12s5.383,12,12,12,12-5.383,12-12S18.617,0,12,0Zm0,21c-4.962,0-9-4.037-9-9S7.038,3,12,3s9,4.038,9,9-4.038,9-9,9Zm4-9c0,2.209-1.791,4-4,4s-4-1.791-4-4,1.791-4,4-4,4,1.791,4,4Z" }
            }
        }
    }
}

#[component]
fn Loading(#[props(default)] class: String) -> Element {
    rsx! {
        button { tabindex: "-1", class: "btn btn-square btn-ghost {class}",
            span { class: "loading loading-ring loading-lg" }
        }
    }
}
