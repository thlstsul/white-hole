use dioxus::prelude::*;

#[component]
pub fn BodyArea(
    #[props(default)] class: String,
    value: Signal<String>,
    contenteditable: bool,
) -> Element {
    rsx! {
        div {
            class: "body-area textarea textarea-ghost textarea-neutral {class}",
            contenteditable: contenteditable.to_string(),
            oninput: move |e| {
                value.set(e.value());
            },

            {value}
        }
    }
}
