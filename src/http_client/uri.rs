use dioxus::prelude::*;

#[component]
pub fn UriInput(#[props(default)] class: String, value: Signal<String>) -> Element {
    rsx! {
        input {
            value,
            r#type: "text",
            class: "uri-input input input-ghost input-neutral {class}",
            placeholder: "Uri",
            oninput: move |e| {
                value.set(e.value());
            },
            onfocusout: move |_| {
                let mut input = value.write();
                if !input.is_empty() && !input.starts_with("http://")
                    && !input.starts_with("https://")
                {
                    *input = format!("http://{}", *input);
                }
            },
        }
    }
}
