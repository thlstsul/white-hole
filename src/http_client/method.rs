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
                option {
                    value: *method,
                    selected: *method == value(),
                    class: "bg-base-100 text-base-content",
                    "{method}"
                }
            }
        }
    }
}

const METHODS: &[&str] = &[
    "POST", "GET", "CONNECT", "DELETE", "HEAD", "OPTIONS", "PATCH", "PUT", "TRACE",
];
