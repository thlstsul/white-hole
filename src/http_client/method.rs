use dioxus::prelude::*;

#[component]
pub fn MethodSelect(#[props(default)] class: String, value: Signal<String>) -> Element {
    if value.is_empty() {
        value.set(String::from("POST"));
    }

    rsx! {
        select {
            value,
            class: "method-select select select-ghost select-neutral w-32 {class}",
            onchange: move |e| {
                value.set(e.value());
            },

            for method in METHODS {
                if *method == value() {
                    option {
                        value: *method,
                        selected: true,
                        class: "bg-base-100 text-base-content",
                        "{method}"
                    }
                } else {
                    option {
                        value: *method,
                        class: "bg-base-100 text-base-content",
                        "{method}"
                    }
                }
            }
        }
    }
}

const METHODS: &[&str] = &[
    "POST", "GET", "CONNECT", "DELETE", "HEAD", "OPTIONS", "PATCH", "PUT", "TRACE",
];
