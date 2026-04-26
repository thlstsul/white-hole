use dioxus::prelude::*;

#[component]
pub fn MethodSelect(#[props(default)] class: String, value: Signal<String>) -> Element {
    rsx! {
        select {
            value,
            class: "method-select select select-ghost select-neutral w-32 {class}",
            onchange: move |e| {
                value.set(e.value());
            },

            for method in METHODS {
                option { value: *method, "{method}" }
            }
        }
    }
}

const METHODS: &[&str] = &[
    "CONNECT", "DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT", "TRACE",
];
